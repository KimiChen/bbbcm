//! Restricted identity services. The caller owns the transaction, account authorization,
//! complete lock plan and commit. There is no promotion or invitation-grant endpoint.
use crate::{
    contract::{
        ContributionKind, INVITATION_POLICY_VERSION, InvitationAuthorization, TierInputs,
        TrustError, TrustLevel, TrustSnapshot,
    },
    risk::{
        RISK_EVENT_VERSION, RiskEvent, RiskEventKind, RiskPolicy, RiskSignals, TrustedRiskContext,
    },
};
use chrono::{DateTime, Utc};
use platform::{
    identifiers::SchemaRenderer,
    transaction::{LockPlan, LockTarget},
};
use platform_db::transaction::TransactionConnection;

fn valid_id(id: i64) -> Result<(), TrustError> {
    if id > 0 {
        Ok(())
    } else {
        Err(TrustError::InvalidInput)
    }
}
fn canonical(now: DateTime<Utc>) -> Result<DateTime<Utc>, TrustError> {
    if now < DateTime::UNIX_EPOCH {
        return Err(TrustError::InvalidInput);
    }
    platform::db::decode_time(&platform::db::encode_time(now)).map_err(|_| TrustError::InvalidInput)
}

pub async fn register_initial(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    invitation_id: i64,
    inviter_id: i64,
    now: DateTime<Utc>,
) -> Result<TrustSnapshot, TrustError> {
    valid_id(user_id)?;
    valid_id(invitation_id)?;
    valid_id(inviter_id)?;
    if user_id == inviter_id {
        return Err(TrustError::InvalidInput);
    }
    crate::repo::insert_initial(
        tx,
        renderer,
        user_id,
        invitation_id,
        inviter_id,
        canonical(now)?,
    )
    .await?;
    crate::repo::load_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::CorruptData)
}

pub async fn load_identity_snapshot(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<TrustSnapshot>, TrustError> {
    valid_id(user_id)?;
    crate::repo::load_snapshot(tx, renderer, user_id).await
}

pub async fn mark_email_verified(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<TrustSnapshot, TrustError> {
    let now = canonical(now)?;
    let before = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    let mut targets = vec![
        LockTarget::new("trust", "identity_snapshots", before.id).ok_or(TrustError::Unavailable)?,
    ];
    if let Some(id) = crate::repo::invite_use_id(tx, renderer, user_id).await? {
        targets.push(LockTarget::new("trust", "invite_uses", id).ok_or(TrustError::Unavailable)?);
    }
    for target in LockPlan::new(targets).targets() {
        tx.lock(renderer, target).await?;
    }
    let current = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    if now < current.updated_at {
        return Err(TrustError::InvalidInput);
    }
    if current.email_verified
        && current.invitation_allowed
        && current.policy_version == INVITATION_POLICY_VERSION
    {
        // 邀请资格已经齐备，无需重复写入。但仍要走一次授级：DEC-054 生效之前留下的
        // 「已验证但仍为 pending」的行，必须在这里收敛到 T0，否则会永远停在 pending。
        return grant_verified_tier(tx, renderer, current, now).await;
    }
    // DEC-051：已验证邮箱即取得邀请资格，与信任等级无关。
    // DEC-054 在此基础上追加：验证通过同时自动授予 T0。
    // T1 不在这里授予，它由 `evaluate_tier` 的确定性规则单独评估。
    crate::repo::confirm_email(tx, renderer, &current, now).await?;
    let confirmed = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::CorruptData)?;
    grant_verified_tier(tx, renderer, confirmed, now).await
}

/// DEC-054：邮箱验证只授予 T0，不附带任何其他权限。
///
/// 幂等与不降级：已经是 T0 或 T1 的账号原样返回，重复验证不会改写等级，也不会把 T1 降回 T0。
/// 未验证的账号直接返回，绝不越过验证前置条件授级。
///
/// 锁：调用方（`mark_email_verified`）已经持有 `trust.identity_snapshots` 行锁，
/// 这里只做版本受控的更新，不再取新锁。
async fn grant_verified_tier(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    snapshot: TrustSnapshot,
    now: DateTime<Utc>,
) -> Result<TrustSnapshot, TrustError> {
    if !snapshot.email_verified() || snapshot.level() != TrustLevel::Pending {
        return Ok(snapshot);
    }
    crate::repo::set_level(
        tx,
        renderer,
        &snapshot,
        TrustLevel::T0,
        "email_verified_tier_t0",
        now,
    )
    .await?;
    load_identity_snapshot(tx, renderer, snapshot.user_id())
        .await?
        .ok_or(TrustError::CorruptData)
}

/// DEC-054 的 T1 授级门槛常量，规则版本见 `TIER_RULE_VERSION`。
///
/// 三个条件必须同时满足：账号已是 T0 且邮箱已验证、账号存在满 2 个自然日（UTC）、
/// 已计数的有效贡献不少于 `T1_CONTRIBUTIONS` 条，并且当前无生效处罚。
///
/// 「满 2 个自然日」的精确读法是 `now >= 注册日当天 UTC 零点 + 2 天`，即跨过两个 UTC 零点，
/// 与 DEC-048／DEC-054 的配额日界线口径一致，也对应 `01` §3.2 里「跨天活跃」的表述。
/// 采用自然日而非 48 小时，是为了让同一批注册用户在同一时刻同时达标，便于解释与复算。
pub const T1_ACCOUNT_DAYS: i64 = 2;
pub const T1_CONTRIBUTIONS: i64 = 5;

fn t1_eligible(
    snapshot: &TrustSnapshot,
    contributions: i64,
    inputs: TierInputs,
    now: DateTime<Utc>,
) -> Result<bool, TrustError> {
    if inputs.active_penalty
        || !snapshot.email_verified()
        || snapshot.level() != TrustLevel::T0
        || contributions < T1_CONTRIBUTIONS
    {
        return Ok(false);
    }
    let opened = snapshot
        .created_at()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or(TrustError::InvalidInput)?
        .and_utc()
        .checked_add_signed(
            chrono::Duration::try_days(T1_ACCOUNT_DAYS).ok_or(TrustError::InvalidInput)?,
        )
        .ok_or(TrustError::InvalidInput)?;
    Ok(now >= opened)
}

/// 评估 T1 规则，达标则在信任快照行锁下应用授级。
///
/// 调用时机：必须在账号根锁之后、社区（community）各表锁之前。本函数会对
/// `trust.identity_snapshots` 重取一次同一目标的行锁——与「上一把锁」相等而非更靠前，
/// 因此不违反 DEC-052 约定的锁序；进入追加段之后则不能再调用。
///
/// 行为约束：只升不降（永不把 T1 降回 T0）、不授予 T0（那是验证路径的职责）、
/// 不读取任何其他领域的仓储。「是否存在生效处罚」这类跨域事实由调用方通过 `inputs` 注入，
/// 规则本身与版本号留在 trust 内，保证可解释、可重算、可审计（`TRUST-002`）。
pub async fn evaluate_tier(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    inputs: TierInputs,
    now: DateTime<Utc>,
) -> Result<TrustSnapshot, TrustError> {
    let now = canonical(now)?;
    let snapshot = lock_content_snapshot(tx, renderer, user_id).await?;
    let contributions = crate::repo::count_contributions(tx, renderer, user_id).await?;
    if !t1_eligible(&snapshot, contributions, inputs, now)? {
        return Ok(snapshot);
    }
    crate::repo::set_level(
        tx,
        renderer,
        &snapshot,
        TrustLevel::T1,
        "tier_rule_v1_days_contributions",
        now,
    )
    .await?;
    load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::CorruptData)
}

/// 在发布事务内记录一条合格的有效贡献。
///
/// 有效性由内容领域按 DEC-054 判定后才会调用到这里，trust 只负责落库，不重复判定业务语义。
/// 同一 `post_id` 重复调用不会重复计数（用于命令重放与幂等重试）。
pub async fn record_contribution(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    kind: ContributionKind,
    post_id: i64,
    body_chars: i64,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    valid_id(user_id)?;
    valid_id(post_id)?;
    if body_chars < 0 {
        return Err(TrustError::InvalidInput);
    }
    crate::repo::insert_contribution(
        tx,
        renderer,
        user_id,
        kind,
        post_id,
        body_chars,
        canonical(now)?,
    )
    .await
}

/// 在删除所在的同一事务内把一条贡献置为不计数。
///
/// 事实行本身保留，只把 `counted` 置否，这样贡献总数始终可以从追加历史重算，
/// 而不是靠删行「抹掉历史」。
pub async fn withdraw_contribution(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    post_id: i64,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    valid_id(post_id)?;
    crate::repo::withdraw_contribution(tx, renderer, post_id, canonical(now)?).await
}

pub async fn authorize_invitation(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<InvitationAuthorization, TrustError> {
    let found = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    let target =
        LockTarget::new("trust", "identity_snapshots", found.id).ok_or(TrustError::Unavailable)?;
    tx.lock(renderer, &target).await?;
    let snapshot = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    if !snapshot.email_verified {
        return Ok(InvitationAuthorization::EmailUnverified);
    }
    if snapshot.invitation_allowed && snapshot.policy_version > 0 {
        return Ok(InvitationAuthorization::Allowed {
            policy_version: snapshot.policy_version,
        });
    }
    Ok(InvitationAuthorization::NotGranted)
}

/// Read the current stored trust facts, then evaluate a separately approved policy.
/// This function appends the decision in the same transaction and never commits.
/// Invoke after all mutable domain locks; Outbox/analytics appends may follow it.
#[allow(clippy::too_many_arguments)]
pub async fn record_identity_event(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event_id: &str,
    actor_id: Option<i64>,
    kind: RiskEventKind,
    signals: RiskSignals,
    policy: &dyn RiskPolicy,
    now: DateTime<Utc>,
) -> Result<RiskEvent, TrustError> {
    if !platform::transaction::validate_operation_id(event_id) {
        return Err(TrustError::InvalidInput);
    }
    let snapshot = match actor_id {
        Some(id) => Some(
            load_identity_snapshot(tx, renderer, id)
                .await?
                .ok_or(TrustError::NotFound)?,
        ),
        None => None,
    };
    let context = TrustedRiskContext {
        actor_id,
        snapshot,
        kind,
        signals,
    };
    let decision = policy.assess(&context)?;
    let event = RiskEvent {
        event_version: RISK_EVENT_VERSION,
        event_id: event_id.into(),
        actor_id,
        kind,
        occurred_at: canonical(now)?,
        signals: context.signals,
        decision,
    };
    crate::repo::append_risk(tx, renderer, &event).await?;
    Ok(event)
}

#[cfg(test)]
mod tests;

/// Called under the identity user root lock, before any community lock.
/// No grant, default level or invitation-derived publishing permission is created.
pub async fn lock_content_snapshot(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<TrustSnapshot, TrustError> {
    let snapshot = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    tx.lock(
        renderer,
        &LockTarget::new("trust", "identity_snapshots", snapshot.id)
            .ok_or(TrustError::Unavailable)?,
    )
    .await?;
    load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)
}

/// Append a server-evaluated content decision after all mutable locks are held.
/// The owning command owns policy evaluation and commit; this is not a public grant API.
#[allow(clippy::too_many_arguments)]
pub async fn record_content_decision(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event_id: &str,
    user_id: i64,
    kind: RiskEventKind,
    signals: RiskSignals,
    decision: crate::risk::RiskDecision,
    now: DateTime<Utc>,
) -> Result<RiskEvent, TrustError> {
    valid_id(user_id)?;
    if !platform::transaction::validate_operation_id(event_id)
        || !matches!(
            kind,
            RiskEventKind::Topic
                | RiskEventKind::Reply
                | RiskEventKind::Edit
                | RiskEventKind::Delete
        )
    {
        return Err(TrustError::InvalidInput);
    }
    let snapshot = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    if decision.allowed() && !snapshot.email_verified() {
        return Err(TrustError::InvalidInput);
    }
    let event = RiskEvent {
        event_version: RISK_EVENT_VERSION,
        event_id: event_id.into(),
        actor_id: Some(user_id),
        kind,
        occurred_at: canonical(now)?,
        signals,
        decision,
    };
    crate::repo::append_risk(tx, renderer, &event).await?;
    Ok(event)
}
