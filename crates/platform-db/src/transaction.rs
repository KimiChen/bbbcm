//! 事务所有权留在应用服务；Repository 只获得不可提交的借用连接。
//! sqlx TransactionManager 维护驱动事务深度；RAII guard 在取消/未知结果时回滚并丢弃连接。

use crate::Pool;
use chrono::{DateTime, Utc};
use platform::db::{self, Backend};
use platform::identifiers::SchemaRenderer;
use platform::transaction::{DbErrorClass, LockTarget, classify_database_code};
use sqlx::{Row, TransactionManager};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransactionError {
    #[error("事务输入或受信标识符非法")]
    InvalidInput,
    #[error("数据库事务失败（{0:?}，底层数据已脱敏）")]
    Database(DbErrorClass),
    #[error("实体版本冲突")]
    VersionConflict,
    #[error("幂等键已用于不同载荷")]
    IdempotencyConflict,
    #[error("幂等响应已过期；必须由业务唯一事实裁决，不能自动重放")]
    ResponseExpired,
    #[error("锁顺序违反已冻结总序；应回滚并重新形成完整锁计划")]
    LockOrder,
    #[error("数据库数据损坏或超出共享合同")]
    CorruptData,
    #[error("提交结果未知；仅允许凭原身份、作用域与幂等键查询")]
    CommitUnknown,
}
impl From<platform::identifiers::SchemaError> for TransactionError {
    fn from(_: platform::identifiers::SchemaError) -> Self {
        Self::InvalidInput
    }
}
impl From<platform::db::RenderError> for TransactionError {
    fn from(_: platform::db::RenderError) -> Self {
        Self::InvalidInput
    }
}

pub fn classify_error(backend: Backend, error: &sqlx::Error) -> TransactionError {
    let class = match error {
        sqlx::Error::Database(db) => db
            .code()
            .map(|code| classify_database_code(backend, &code))
            .unwrap_or(DbErrorClass::Other),
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => DbErrorClass::Connection,
        _ => DbErrorClass::Other,
    };
    TransactionError::Database(class)
}

#[derive(Clone)]
pub enum DbValue {
    Text(String),
    I64(i64),
    Bool(bool),
    Time(DateTime<Utc>),
}
impl From<&str> for DbValue {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}
impl From<String> for DbValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<i64> for DbValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

pub enum DbRow {
    Postgres(sqlx::postgres::PgRow),
    Sqlite(sqlx::sqlite::SqliteRow),
}
impl std::fmt::Debug for DbRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbRow([redacted])")
    }
}
impl DbRow {
    pub fn text(&self, column: &str) -> Result<String, TransactionError> {
        match self {
            Self::Postgres(row) => row.try_get(column),
            Self::Sqlite(row) => row.try_get(column),
        }
        .map_err(|_| TransactionError::CorruptData)
    }
    pub fn i64(&self, column: &str) -> Result<i64, TransactionError> {
        match self {
            Self::Postgres(row) => row
                .try_get(column)
                .or_else(|_| row.try_get::<i32, _>(column).map(i64::from)),
            Self::Sqlite(row) => row.try_get(column),
        }
        .map_err(|_| TransactionError::CorruptData)
    }
    pub fn boolean(&self, column: &str) -> Result<bool, TransactionError> {
        match self {
            Self::Postgres(row) => row
                .try_get(column)
                .map_err(|_| TransactionError::CorruptData),
            Self::Sqlite(_) => match self.i64(column)? {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(TransactionError::CorruptData),
            },
        }
    }
    pub fn time(&self, column: &str) -> Result<DateTime<Utc>, TransactionError> {
        match self {
            Self::Postgres(row) => row
                .try_get(column)
                .map_err(|_| TransactionError::CorruptData),
            Self::Sqlite(_) => {
                let value = self.text(column)?;
                let decoded = db::decode_time(&value).map_err(|_| TransactionError::CorruptData)?;
                if db::encode_time(decoded) != value {
                    return Err(TransactionError::CorruptData);
                }
                Ok(decoded)
            }
        }
    }
}

enum OwnedTransaction {
    Postgres(sqlx::pool::PoolConnection<sqlx::Postgres>),
    Sqlite(sqlx::pool::PoolConnection<sqlx::Sqlite>),
}

#[derive(Default)]
struct LockProgress {
    last: Option<LockTarget>,
    append_only: bool,
    idempotency_started: bool,
}

type PgManager = <sqlx::Postgres as sqlx::Database>::TransactionManager;
type SqliteManager = <sqlx::Sqlite as sqlx::Database>::TransactionManager;

pub struct DbTransaction {
    inner: OwnedTransaction,
    locks: LockProgress,
    finished: bool,
}
impl DbTransaction {
    pub async fn begin(pool: &Pool) -> Result<Self, TransactionError> {
        let backend = pool.backend();
        let mut result = Self {
            inner: match pool {
                Pool::Postgres(pool) => OwnedTransaction::Postgres(
                    pool.acquire()
                        .await
                        .map_err(|e| classify_error(backend, &e))?,
                ),
                Pool::Sqlite(pool) => OwnedTransaction::Sqlite(
                    pool.acquire()
                        .await
                        .map_err(|e| classify_error(backend, &e))?,
                ),
            },
            locks: LockProgress::default(),
            finished: false,
        };
        // guard 先于 BEGIN future 存在；开始阶段被取消也不会把半打开连接归池。
        let statement = Some(std::borrow::Cow::Borrowed(db::begin_write_sql(backend)));
        match &mut result.inner {
            OwnedTransaction::Postgres(conn) => PgManager::begin(conn, statement).await,
            OwnedTransaction::Sqlite(conn) => SqliteManager::begin(conn, statement).await,
        }
        .map_err(|e| classify_error(backend, &e))?;
        if let Some(setup) = db::transaction_setup_sql(backend) {
            result.connection().execute_raw(setup, &[]).await?;
        }
        Ok(result)
    }

    pub fn connection(&mut self) -> TransactionConnection<'_> {
        TransactionConnection {
            inner: match &mut self.inner {
                OwnedTransaction::Postgres(conn) => BorrowedConnection::Postgres(conn),
                OwnedTransaction::Sqlite(conn) => BorrowedConnection::Sqlite(conn),
            },
            locks: &mut self.locks,
        }
    }

    pub async fn rollback(mut self) -> Result<(), TransactionError> {
        let backend = self.connection().backend();
        self.finish_driver(false)
            .await
            .map_err(|e| classify_error(backend, &e))?;
        self.finished = true;
        Ok(())
    }

    async fn finish_driver(&mut self, commit: bool) -> Result<(), sqlx::Error> {
        match &mut self.inner {
            OwnedTransaction::Postgres(conn) => {
                if commit {
                    PgManager::commit(conn).await
                } else {
                    PgManager::rollback(conn).await
                }
            }
            OwnedTransaction::Sqlite(conn) => {
                if commit {
                    SqliteManager::commit(conn).await
                } else {
                    SqliteManager::rollback(conn).await
                }
            }
        }
    }

    /// I/O 等提交错误不能推断服务器结果；任何失败都由 Drop 丢弃原连接。
    pub async fn commit(mut self) -> Result<(), TransactionError> {
        let backend = self.connection().backend();
        self.finish_driver(true).await.map_err(|error| {
            let classified = classify_error(backend, &error);
            if backend == Backend::Postgres
                && matches!(
                    classified,
                    TransactionError::Database(
                        DbErrorClass::Serialization | DbErrorClass::Deadlock
                    )
                )
            {
                classified
            } else {
                TransactionError::CommitUnknown
            }
        })?;
        self.finished = true;
        Ok(())
    }

    /// 仅测试适配：实际完成存储动作，随后模拟回执丢失，走同一 RAII 丢弃路径。
    #[cfg(test)]
    pub(crate) async fn simulate_lost_ack(
        mut self,
        persisted: bool,
    ) -> Result<(), TransactionError> {
        self.finish_driver(persisted)
            .await
            .map_err(|_| TransactionError::CommitUnknown)?;
        // 刻意不授予 finished；Unknown 的连接即使健康也不能再次归池。
        Err(TransactionError::CommitUnknown)
    }
}
impl Drop for DbTransaction {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        match &mut self.inner {
            OwnedTransaction::Postgres(conn) => {
                PgManager::start_rollback(conn);
                conn.close_on_drop();
            }
            OwnedTransaction::Sqlite(conn) => {
                SqliteManager::start_rollback(conn);
                conn.close_on_drop();
            }
        }
    }
}

enum BorrowedConnection<'a> {
    Postgres(&'a mut sqlx::PgConnection),
    Sqlite(&'a mut sqlx::SqliteConnection),
}
pub struct TransactionConnection<'a> {
    inner: BorrowedConnection<'a>,
    locks: &'a mut LockProgress,
}

fn pg_query<'q>(
    sql: &'q str,
    parameters: &[DbValue],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let mut query = sqlx::query(sql);
    for value in parameters {
        query = match value {
            DbValue::Text(v) => query.bind(v.clone()),
            DbValue::I64(v) => query.bind(*v),
            DbValue::Bool(v) => query.bind(*v),
            DbValue::Time(v) => query.bind(*v),
        };
    }
    query
}
fn sqlite_query<'q>(
    sql: &'q str,
    parameters: &[DbValue],
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    let mut query = sqlx::query(sql);
    for value in parameters {
        query = match value {
            DbValue::Text(v) => query.bind(v.clone()),
            DbValue::I64(v) => query.bind(*v),
            DbValue::Bool(v) => query.bind(i64::from(*v)),
            DbValue::Time(v) => query.bind(db::encode_time(*v)),
        };
    }
    query
}

impl TransactionConnection<'_> {
    pub fn backend(&self) -> Backend {
        match self.inner {
            BorrowedConnection::Postgres(_) => Backend::Postgres,
            BorrowedConnection::Sqlite(_) => Backend::Sqlite,
        }
    }

    pub(crate) fn start_idempotency(&mut self) -> Result<(), TransactionError> {
        if self.locks.idempotency_started || self.locks.last.is_some() || self.locks.append_only {
            return Err(TransactionError::LockOrder);
        }
        self.locks.idempotency_started = true;
        Ok(())
    }

    /// 低层共享 SQL 执行入口；标识符仍须调用受信 SchemaRenderer，值只经 bind。
    pub async fn execute(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<u64, TransactionError> {
        validate_repository_sql(sql)?;
        self.execute_raw(sql, parameters).await
    }

    async fn execute_raw(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<u64, TransactionError> {
        let backend = self.backend();
        let sql = db::render_sql(backend, sql)?;
        match &mut self.inner {
            BorrowedConnection::Postgres(c) => pg_query(&sql, parameters)
                .execute(&mut **c)
                .await
                .map(|r| r.rows_affected()),
            BorrowedConnection::Sqlite(c) => sqlite_query(&sql, parameters)
                .execute(&mut **c)
                .await
                .map(|r| r.rows_affected()),
        }
        .map_err(|e| classify_error(backend, &e))
    }

    pub async fn fetch_all(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<Vec<DbRow>, TransactionError> {
        validate_repository_sql(sql)?;
        let backend = self.backend();
        let sql = db::render_sql(backend, sql)?;
        match &mut self.inner {
            BorrowedConnection::Postgres(c) => pg_query(&sql, parameters)
                .fetch_all(&mut **c)
                .await
                .map(|rows| rows.into_iter().map(DbRow::Postgres).collect()),
            BorrowedConnection::Sqlite(c) => sqlite_query(&sql, parameters)
                .fetch_all(&mut **c)
                .await
                .map(|rows| rows.into_iter().map(DbRow::Sqlite).collect()),
        }
        .map_err(|e| classify_error(backend, &e))
    }

    pub async fn fetch_optional(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<Option<DbRow>, TransactionError> {
        let mut rows = self.fetch_all(sql, parameters).await?;
        if rows.len() > 1 {
            return Err(TransactionError::CorruptData);
        }
        Ok(rows.pop())
    }

    pub async fn last_insert_id(&mut self) -> Result<i64, TransactionError> {
        let row = self
            .fetch_optional(db::last_insert_id_sql(self.backend()), &[])
            .await?
            .ok_or(TransactionError::CorruptData)?;
        let id = row.i64("id")?;
        if id <= 0 {
            return Err(TransactionError::CorruptData);
        }
        Ok(id)
    }

    pub async fn lock(
        &mut self,
        renderer: &SchemaRenderer,
        target: &LockTarget,
    ) -> Result<(), TransactionError> {
        if self.locks.append_only || self.locks.last.as_ref().is_some_and(|last| target < last) {
            return Err(TransactionError::LockOrder);
        }
        let table = renderer.table(self.backend(), target.schema(), target.table())?;
        let sql = db::lock_sql(
            self.backend(),
            &format!("SELECT id FROM {table} WHERE id = ?"),
            false,
        );
        self.fetch_optional(&sql, &[DbValue::I64(target.id())])
            .await?
            .ok_or(TransactionError::VersionConflict)?;
        self.locks.last = Some(target.clone());
        Ok(())
    }

    /// 进入幂等结果、Outbox、审计追加段后，禁止补取较早领域锁。
    pub fn enter_append_phase(&mut self) {
        self.locks.append_only = true;
    }

    pub async fn execute_versioned(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<(), TransactionError> {
        match self.execute(sql, parameters).await? {
            1 => Ok(()),
            0 => Err(TransactionError::VersionConflict),
            _ => Err(TransactionError::CorruptData),
        }
    }

    /// PG 唯一冲突必须先恢复保存点，不能直接在 aborted 事务中 UPDATE。
    pub async fn insert_or_update(
        &mut self,
        insert: &str,
        insert_parameters: &[DbValue],
        update: &str,
        update_parameters: &[DbValue],
    ) -> Result<u64, TransactionError> {
        self.execute_raw("SAVEPOINT bcm_platform_upsert", &[])
            .await?;
        let inserted = self.execute(insert, insert_parameters).await;
        match inserted {
            Ok(rows) => {
                self.execute_raw("RELEASE SAVEPOINT bcm_platform_upsert", &[])
                    .await?;
                Ok(rows)
            }
            Err(error) => {
                self.execute_raw("ROLLBACK TO SAVEPOINT bcm_platform_upsert", &[])
                    .await?;
                self.execute_raw("RELEASE SAVEPOINT bcm_platform_upsert", &[])
                    .await?;
                if error == TransactionError::Database(DbErrorClass::UniqueConflict) {
                    self.execute(update, update_parameters).await
                } else {
                    Err(error)
                }
            }
        }
    }

    /// 为幂等 INSERT 提供保存点保护；冲突后保留当前事务供回读核验。
    pub async fn insert_if_absent(
        &mut self,
        sql: &str,
        parameters: &[DbValue],
    ) -> Result<bool, TransactionError> {
        self.execute_raw("SAVEPOINT bcm_platform_insert", &[])
            .await?;
        match self.execute(sql, parameters).await {
            Ok(1) => {
                self.execute_raw("RELEASE SAVEPOINT bcm_platform_insert", &[])
                    .await?;
                Ok(true)
            }
            result => {
                self.execute_raw("ROLLBACK TO SAVEPOINT bcm_platform_insert", &[])
                    .await?;
                self.execute_raw("RELEASE SAVEPOINT bcm_platform_insert", &[])
                    .await?;
                match result {
                    Err(TransactionError::Database(DbErrorClass::UniqueConflict)) => Ok(false),
                    Err(e) => Err(e),
                    _ => Err(TransactionError::CorruptData),
                }
            }
        }
    }
}

fn validate_repository_sql(sql: &str) -> Result<(), TransactionError> {
    // 不接受注释前缀或多语句；仓库 SQL 采用固定 DML 字面量，内容全部 bind。
    // 即使借用者试图直接 COMMIT，事务生命周期也只能通过所有者结束。
    let keyword = sql.split_ascii_whitespace().next().unwrap_or("");
    if !["SELECT", "INSERT", "UPDATE", "DELETE", "WITH"]
        .iter()
        .any(|allowed| keyword.eq_ignore_ascii_case(allowed))
        || sql.contains(';')
    {
        return Err(TransactionError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
