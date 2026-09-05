//! 应用服务调用模板：幂等、完整锁计划、领域写、结果与提交一次完成。

use crate::Pool;
use crate::idempotency_repo::{self, Reservation};
use crate::transaction::{DbTransaction, TransactionConnection, TransactionError};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform::transaction::{
    CommandIdentity, CommitOutcome, DbErrorClass, LockPlan, RetryBudget, StoredResponse,
};
use std::future::Future;
use std::pin::Pin;

pub type CommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<StoredResponse, TransactionError>> + Send + 'a>>;

#[derive(Debug, PartialEq, Eq)]
pub struct CommandResult {
    pub operation_id: String,
    pub response: StoredResponse,
    pub replayed: bool,
}

/// 业务闭包只能做当前数据库事务内的写；外部副作用必须写为 Outbox。
/// 服务必须在闭包里重读所有可变授权事实；该模板不替代身份/权限系统。
pub async fn execute_command<F>(
    pool: &Pool,
    renderer: &SchemaRenderer,
    command: &CommandIdentity,
    locks: &LockPlan,
    now: DateTime<Utc>,
    work: F,
) -> Result<CommitOutcome<CommandResult>, TransactionError>
where
    F: for<'a, 'c> FnMut(&'a mut TransactionConnection<'c>) -> CommandFuture<'a>,
{
    execute_inner(
        pool,
        renderer,
        command,
        locks,
        now,
        work,
        CommitTestMode::Normal,
    )
    .await
}

#[derive(Clone, Copy)]
enum CommitTestMode {
    Normal,
    #[cfg(test)]
    LoseAcknowledgementAfterCommit,
    #[cfg(test)]
    LoseAcknowledgementAfterRollback,
}

async fn execute_inner<F>(
    pool: &Pool,
    renderer: &SchemaRenderer,
    command: &CommandIdentity,
    locks: &LockPlan,
    now: DateTime<Utc>,
    mut work: F,
    commit_mode: CommitTestMode,
) -> Result<CommitOutcome<CommandResult>, TransactionError>
where
    F: for<'a, 'c> FnMut(&'a mut TransactionConnection<'c>) -> CommandFuture<'a>,
{
    if !command.is_valid() {
        return Err(TransactionError::InvalidInput);
    }
    let mut budget = RetryBudget::default();
    loop {
        let mut tx = match DbTransaction::begin(pool).await {
            Ok(tx) => tx,
            Err(error) => {
                // 开始事务未成功，调用者尚未执行任何写入。
                if can_retry(&mut budget, pool, &error, true) {
                    backoff(&budget).await;
                    continue;
                }
                return Err(error);
            }
        };
        let attempt = async {
            let mut conn = tx.connection();
            match idempotency_repo::reserve(&mut conn, renderer, command, now).await? {
                Reservation::Replay {
                    operation_id,
                    response,
                } => Ok(CommandResult {
                    operation_id,
                    response,
                    replayed: true,
                }),
                Reservation::New => {
                    for target in locks.targets() {
                        conn.lock(renderer, target).await?;
                    }
                    let response = work(&mut conn).await?;
                    idempotency_repo::complete(&mut conn, renderer, command, &response, now)
                        .await?;
                    Ok(CommandResult {
                        operation_id: command.operation_id.clone(),
                        response,
                        replayed: false,
                    })
                }
            }
        }
        .await;
        let result = match attempt {
            Ok(result) => result,
            Err(error) => {
                let rollback_confirmed = tx.rollback().await.is_ok();
                if can_retry(&mut budget, pool, &error, rollback_confirmed) {
                    backoff(&budget).await;
                    continue;
                }
                if !rollback_confirmed {
                    return Err(TransactionError::Database(DbErrorClass::Connection));
                }
                return Err(error);
            }
        };
        if result.replayed {
            tx.rollback().await?;
            return Ok(CommitOutcome::Committed(result));
        }
        let committed = match commit_mode {
            CommitTestMode::Normal => tx.commit().await,
            #[cfg(test)]
            CommitTestMode::LoseAcknowledgementAfterCommit => tx.simulate_lost_ack(true).await,
            #[cfg(test)]
            CommitTestMode::LoseAcknowledgementAfterRollback => tx.simulate_lost_ack(false).await,
        };
        match committed {
            Ok(()) => {
                return Ok(CommitOutcome::Committed(result));
            }
            Err(TransactionError::CommitUnknown) => {
                return Ok(CommitOutcome::Unknown {
                    operation_id: command.operation_id.clone(),
                });
            }
            Err(error) => {
                // commit 仅为服务端已确认事务拒绝的 40001/40P01 返回分类错误。
                if can_retry(&mut budget, pool, &error, true) {
                    backoff(&budget).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
}

fn can_retry(
    budget: &mut RetryBudget,
    pool: &Pool,
    error: &TransactionError,
    confirmed: bool,
) -> bool {
    if let TransactionError::Database(class) = error {
        budget.take(pool.backend(), *class, true, confirmed)
    } else {
        false
    }
}
async fn backoff(budget: &RetryBudget) {
    tokio::time::sleep(std::time::Duration::from_millis(10_u64 << budget.retries())).await;
}

#[cfg(test)]
mod tests;
