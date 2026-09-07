//! `SOCIAL-002`／`SOCIAL-003` 正向反馈的领域服务。
//!
//! 边界：本模块**不判断谁可以反馈**，也不读内容表。目标是否存在、是否可见、
//! 作者是谁，全部由调用方以 `FeedbackSubject` 传入——它必须在**同一个事务内**
//! 重读这些事实。理由与 content 领域一致：把授权判断留在领域外，可以让
//! 「读到的可见性」与「写入的那一刻」处于同一快照，而不是先查后写的两拍。
use crate::contract::{FeedbackOutcome, FeedbackTarget, InteractionError};
use crate::repo;
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// 由调用方在同一事务内重读得到的受信事实。
///
/// `author_id` 不接受来自请求的自报值：自我反馈的禁止全靠它，请求可控就等于没有约束。
#[derive(Debug, Clone, Copy)]
pub struct FeedbackSubject {
    pub target: FeedbackTarget,
    pub target_id: i64,
    pub author_id: i64,
}

fn validate(user_id: i64, subject: &FeedbackSubject) -> Result<(), InteractionError> {
    if user_id <= 0 || subject.target_id <= 0 || subject.author_id <= 0 {
        return Err(InteractionError::InvalidInput);
    }
    // SOCIAL-003：不能给自己反馈。库里还有一条同义 CHECK 兜底——
    // 这里先判是为了给出明确错误，而不是让用户看到一个笼统的冲突。
    if user_id == subject.author_id {
        return Err(InteractionError::SelfFeedback);
    }
    Ok(())
}

/// 给出一次正向反馈（`SOCIAL-002`）。
///
/// 幂等：已处于「已反馈」状态时不报错，返回 `changed = false`。用户连点两下
/// 不该看到错误，但调用方需要知道有没有真的发生变化（例如要不要发通知）。
///
/// 并发：不做「先查再插」的乐观假设。两路同时插入时，唯一约束会让其中一路
/// 拿到 `Conflict`；调用方重试即读到既有行，走撤销/重做分支。
pub async fn give(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    subject: FeedbackSubject,
    now: DateTime<Utc>,
) -> Result<FeedbackOutcome, InteractionError> {
    validate(user_id, &subject)?;
    let existing = repo::current(c, r, user_id, subject.target, subject.target_id).await?;
    let changed = match existing {
        None => {
            repo::insert(
                c,
                r,
                user_id,
                subject.target,
                subject.target_id,
                subject.author_id,
                now,
            )
            .await?;
            true
        }
        Some((_, false, _)) => false,
        Some((id, true, version)) => {
            repo::set_withdrawn(c, r, id, version, false, now).await?;
            true
        }
    };
    let count = repo::count(c, r, subject.target, subject.target_id).await?;
    Ok(FeedbackOutcome {
        count,
        active: true,
        changed,
    })
}

/// 撤销反馈。
///
/// 撤销不删行，只翻标记：删行会让「撤销后重新反馈」变成又一条新行，唯一约束
/// 反而挡不住；保留行则由同一条记录承载状态变化，也留下痕迹。
pub async fn withdraw(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    subject: FeedbackSubject,
    now: DateTime<Utc>,
) -> Result<FeedbackOutcome, InteractionError> {
    validate(user_id, &subject)?;
    let existing = repo::current(c, r, user_id, subject.target, subject.target_id).await?;
    let changed = match existing {
        // 从未反馈过：撤销是空操作，不报错也不建行。
        None | Some((_, true, _)) => false,
        Some((id, false, version)) => {
            repo::set_withdrawn(c, r, id, version, true, now).await?;
            true
        }
    };
    let count = repo::count(c, r, subject.target, subject.target_id).await?;
    Ok(FeedbackOutcome {
        count,
        active: false,
        changed,
    })
}

/// 某对象当前的有效反馈数。供读路径与 `TOPIC-009` 阈值使用。
pub async fn count(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    target: FeedbackTarget,
    target_id: i64,
) -> Result<i64, InteractionError> {
    if target_id <= 0 {
        return Err(InteractionError::InvalidInput);
    }
    repo::count(c, r, target, target_id).await
}
