//! `NOTIFY-001`／`NOTIFY-002`／`NOTIFY-003` 站内通知。
//!
//! 边界：本模块只管「通知这件事本身」——投递、去重、未读计数、已读。
//! **谁该收到通知由调用方判定**（拉黑、通知偏好、内容可见性都不在这里），
//! 理由与反馈一致：这些事实必须在产生通知的那个事务里重读，放进来反而会
//! 诱导调用方先查后传，产生两拍之间的窗口。
use crate::contract::InteractionError;
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};

type Result<T> = std::result::Result<T, InteractionError>;

/// 未读哨兵。用哨兵时间戳而不是可空列：可空列会让「未读」在 SQL 里变成 NULL
/// 语义，而三库对 NULL 的排序与比较差异正是 `DEC-022` 要避开的。
fn unread() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

/// `NOTIFY-001` 的封闭通知类型。
///
/// `Adopted` 按 `NOTIFY-001` **只预留**：MVP 不产生该类通知，随 P1-A 的
/// `TOPIC-103` 启用。预留在枚举与库约束里而不是等到那时再加，是为了让类型集合
/// 从一开始就完整，避免将来为一个枚举值再动一次表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Reply,
    Mention,
    Quote,
    FavoriteUpdate,
    Adopted,
    Moderation,
    AccountSecurity,
}

impl NotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Mention => "mention",
            Self::Quote => "quote",
            Self::FavoriteUpdate => "favorite_update",
            Self::Adopted => "adopted",
            Self::Moderation => "moderation",
            Self::AccountSecurity => "account_security",
        }
    }

    /// MVP 是否允许产生该类通知。
    ///
    /// 单独一条而不是靠调用方自觉：`Adopted` 一旦被误用，用户会收到一条
    /// 指向尚不存在功能的通知，点进去是空的。
    pub fn produced_in_mvp(self) -> bool {
        self != Self::Adopted
    }
}

/// 通知指向的对象类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Topic,
    Post,
    Case,
    Account,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Post => "post",
            Self::Case => "case",
            Self::Account => "account",
        }
    }
}

/// 一次投递请求。
///
/// `idempotency_key` 由**产生事件的一方**给出，必须能唯一标识「这一个事件对
/// 这一个收件人」。`NOTIFY-003` 要求任务重试不产生重复通知，而重试意味着同一
/// 事件会被投递多次——键相同才挡得住。
#[derive(Debug, Clone)]
pub struct Delivery {
    pub recipient_id: i64,
    pub kind: NotificationKind,
    pub source: SourceKind,
    pub source_id: i64,
    /// 系统事件（审核、账号安全）没有触发者，填 0。
    pub actor_id: i64,
    pub idempotency_key: String,
}

/// 一次投递的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryOutcome {
    /// 真表示本次确实新建了通知；假表示同键通知已存在，本次是重试或并发。
    pub created: bool,
}

fn table(c: &TransactionConnection<'_>, r: &SchemaRenderer, name: &str) -> Result<String> {
    r.table(c.backend(), "interaction", name)
        .map_err(|_| InteractionError::Unavailable)
}

fn validate(d: &Delivery) -> Result<()> {
    if d.recipient_id <= 0
        || d.source_id <= 0
        || d.actor_id < 0
        || d.idempotency_key.is_empty()
        || d.idempotency_key.len() > 200
        || !d.idempotency_key.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(InteractionError::InvalidInput);
    }
    if !d.kind.produced_in_mvp() {
        return Err(InteractionError::InvalidInput);
    }
    // 不给自己发通知：自己回复自己的主题不该产生一条「有人回复了你」。
    if d.actor_id == d.recipient_id {
        return Err(InteractionError::InvalidInput);
    }
    Ok(())
}

/// `NOTIFY-003` 幂等投递。
///
/// 同一 `idempotency_key` 重复投递不会产生第二条通知，也**不报错**——重试是
/// 正常路径，报错会让 Worker 把成功的重试当成失败继续重试下去。
///
/// 并发下靠 `idempotency_key` 的 UNIQUE 兜底：两路同时插入时第二路命中唯一
/// 约束，这里把它翻译成 `created = false`，与「已存在」同义。不做投递前的
/// 先查再插——那在并发下必然漏。
pub async fn deliver(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    delivery: &Delivery,
    now: DateTime<Utc>,
) -> Result<DeliveryOutcome> {
    validate(delivery)?;
    let t = table(c, r, "notifications")?;
    let inserted = c
        .execute(
            &format!(
                "INSERT INTO {t} (recipient_id, kind, source_kind, source_id, actor_id, \
                 idempotency_key, read_at, created_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)"
            ),
            &[
                DbValue::I64(delivery.recipient_id),
                DbValue::Text(delivery.kind.as_str().into()),
                DbValue::Text(delivery.source.as_str().into()),
                DbValue::I64(delivery.source_id),
                DbValue::I64(delivery.actor_id),
                DbValue::Text(delivery.idempotency_key.clone()),
                DbValue::Time(unread()),
                DbValue::Time(now),
            ],
        )
        .await;
    match inserted {
        Ok(_) => Ok(DeliveryOutcome { created: true }),
        Err(e) => match InteractionError::from(e) {
            // 唯一冲突就是「这条通知已经投过了」，属正常结果而非故障。
            InteractionError::Conflict => Ok(DeliveryOutcome { created: false }),
            other => Err(other),
        },
    }
}

/// `NOTIFY-002` 未读计数。
pub async fn unread_count(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    recipient_id: i64,
) -> Result<i64> {
    if recipient_id <= 0 {
        return Err(InteractionError::InvalidInput);
    }
    let t = table(c, r, "notifications")?;
    let row = c
        .fetch_optional(
            &format!("SELECT count(*) AS total FROM {t} WHERE recipient_id = ? AND read_at = ?"),
            &[DbValue::I64(recipient_id), DbValue::Time(unread())],
        )
        .await?
        .ok_or(InteractionError::Unavailable)?;
    Ok(row.i64("total")?)
}

/// `NOTIFY-002` 单条已读。
///
/// `recipient_id` 参与条件，不只用 id：否则拿到别人的通知 ID 就能把它标为已读，
/// 是一次典型的 ID 替换越权。返回是否发生了变化，重复标记不报错。
pub async fn mark_read(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    recipient_id: i64,
    notification_id: i64,
    now: DateTime<Utc>,
) -> Result<bool> {
    if recipient_id <= 0 || notification_id <= 0 {
        return Err(InteractionError::InvalidInput);
    }
    let t = table(c, r, "notifications")?;
    let affected = c
        .execute(
            &format!(
                "UPDATE {t} SET read_at = ?, version = version + 1 \
                 WHERE id = ? AND recipient_id = ? AND read_at = ?"
            ),
            &[
                DbValue::Time(now),
                DbValue::I64(notification_id),
                DbValue::I64(recipient_id),
                DbValue::Time(unread()),
            ],
        )
        .await?;
    Ok(affected == 1)
}

/// `NOTIFY-002` 全部已读。返回本次实际标记的条数。
pub async fn mark_all_read(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    recipient_id: i64,
    now: DateTime<Utc>,
) -> Result<u64> {
    if recipient_id <= 0 {
        return Err(InteractionError::InvalidInput);
    }
    let t = table(c, r, "notifications")?;
    c.execute(
        &format!(
            "UPDATE {t} SET read_at = ?, version = version + 1 \
             WHERE recipient_id = ? AND read_at = ?"
        ),
        &[
            DbValue::Time(now),
            DbValue::I64(recipient_id),
            DbValue::Time(unread()),
        ],
    )
    .await
    .map_err(Into::into)
}
