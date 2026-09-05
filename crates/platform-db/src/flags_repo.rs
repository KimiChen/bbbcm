//! 一条 SELECT 获得完整一致快照；读取失败不得伪装为全关成功。
use crate::Pool;
use platform::flags::{FlagReadError, FlagReader, FlagSnapshot};
use platform::identifiers::SchemaRenderer;
use sqlx::Row;

pub struct DbFlagReader {
    pool: Pool,
    renderer: SchemaRenderer,
}
impl DbFlagReader {
    pub fn new(pool: Pool, renderer: SchemaRenderer) -> Self {
        Self { pool, renderer }
    }
    async fn snapshot(&self) -> Result<FlagSnapshot, FlagReadError> {
        let table = self
            .renderer
            .table(self.pool.backend(), "platform", "feature_flags")
            .map_err(|_| FlagReadError::Corrupt)?;
        let sql = format!("SELECT flag_key, enabled, version FROM {table} ORDER BY flag_key");
        let rows = match &self.pool {
            Pool::Postgres(pool, _) => sqlx::query(&sql)
                .fetch_all(pool)
                .await
                .map_err(|_| FlagReadError::Unavailable)?
                .into_iter()
                .map(|r| {
                    Ok((
                        r.try_get::<String, _>(0)?,
                        r.try_get::<bool, _>(1)?,
                        r.try_get::<i64, _>(2)?,
                    ))
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()
                .map_err(|_| FlagReadError::Corrupt)?,
            Pool::Sqlite(pool, _) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(pool)
                    .await
                    .map_err(|_| FlagReadError::Unavailable)?;
                rows.into_iter()
                    .map(|r| {
                        // SQLite affinity alone does not enforce types; refuse textual/real corruption.
                        let key: String = r.try_get(0).map_err(|_| FlagReadError::Corrupt)?;
                        let enabled: i64 = r.try_get(1).map_err(|_| FlagReadError::Corrupt)?;
                        let version: i64 = r.try_get(2).map_err(|_| FlagReadError::Corrupt)?;
                        if !matches!(enabled, 0 | 1) {
                            return Err(FlagReadError::Corrupt);
                        }
                        Ok((key, enabled == 1, version))
                    })
                    .collect::<Result<Vec<_>, FlagReadError>>()?
            }
        };
        FlagSnapshot::from_rows(rows)
    }
}
impl FlagReader for DbFlagReader {
    fn load(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FlagSnapshot, FlagReadError>> + Send + '_>,
    > {
        Box::pin(self.snapshot())
    }
}

#[cfg(test)]
mod tests;
