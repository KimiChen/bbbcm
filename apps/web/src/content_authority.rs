//! App composition keeps content independent of identity, moderation and trust.
//! Only core services receive the caller-owned transaction; no foreign repo access.
use crate::content_risk::{ContentRiskFactory, RateProtection, TrustedContentRisk};
use chrono::{DateTime, Utc};
use content::contract::*;
use platform::{analytics::TrustedExclusions, identifiers::SchemaRenderer};
use platform_db::transaction::TransactionConnection;
use std::sync::Arc;

/// 内容领域的管理能力到 registry 权限键的映射。
///
/// 映射写在这里而不是内容领域里：内容领域只声明「我需要移动主题这项能力」，
/// 具体对应哪个权限键、哪些角色持有它，是 `DEC-058` registry 的事。
fn permission_key(capability: ManagementCapability) -> &'static str {
    match capability {
        ManagementCapability::MoveTopic => "community.content.move",
    }
}

pub struct ApplicationContentAuthority {
    renderer: SchemaRenderer,
    /// 已批准的内容风控规则版本。
    ///
    /// 取 `None` 时发布整体关闭——装配不完整或配置错误的部署必须落到这个失败关闭态，
    /// 而不是退化成「无风控但可发布」。
    risk_rule_version: Option<i64>,
    /// 作者默认可见性。DEC-054 批准 `PRIV-001`（个人主页列表）与 `PRIV-002`（主题正文）
    /// 默认对未登录访客公开，用户可分别关闭。
    ///
    /// 逐用户的隐私设置入口属于后续波次，因此当前这里承载的是账号级默认值；
    /// 取 `None` 时相应投影一律关闭，绝不臆造一个 Public 事实。
    author_default: Option<Visibility>,
    /// 是否已接入 `DEC-058` 的权限 registry。
    ///
    /// `false` 表示授权底座未装配，所有管理命令整体失败关闭——这不是占位，
    /// 而是显式的未装配态，绝不允许「找不到权限规则就放行」。
    management: bool,
}
impl ApplicationContentAuthority {
    /// 按 DEC-054／DEC-060 已批准事实装配的正常构造入口。
    ///
    /// 注意：仅有本对象还不足以放行发布——`ContentRatePolicy` 也必须装配成功，
    /// 否则风控预检失败关闭。这样未接入 Redis 的部署会停在「不可发布」而不是
    /// 「可发布但无保护」。
    pub fn approved(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            risk_rule_version: Some(crate::content_risk::APPROVED_RATE_VERSION),
            author_default: Some(Visibility::Public),
            // DEC-058 权限 registry 已装配：管理命令走十步授权求值。
            management: true,
        }
    }
    /// 未装配的授权对象，仅测试使用。
    ///
    /// 保留它是为了让「失败关闭」这条路径持续被测试覆盖：没有已批准的规则版本时，
    /// 任何内容命令都必须被拒绝，而不是部分成功。
    #[cfg(test)]
    pub fn new(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            risk_rule_version: None,
            author_default: None,
            management: false,
        }
    }
    #[cfg(test)]
    pub fn synthetic(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            risk_rule_version: Some(1),
            author_default: Some(Visibility::Public),
            management: true,
        }
    }
}
fn failure(error: identity::contract::IdentityError) -> ContentError {
    use identity::contract::IdentityError as E;
    match error {
        E::Csrf | E::Unauthenticated | E::Rejected => ContentError::Forbidden,
        E::PolicyPending => ContentError::PolicyPending,
        _ => ContentError::Unavailable,
    }
}
fn capability(action: Action) -> moderation::contract::ContentCapability {
    use moderation::contract::ContentCapability as C;
    match action {
        Action::CreateTopic => C::Topic,
        Action::CreateReply => C::Reply,
        Action::EditPost => C::Edit,
        Action::DeletePost => C::Delete,
    }
}
impl ContentAuthority for ApplicationContentAuthority {
    /// 把内容领域判定出的有效贡献变更转交 trust 落库。
    ///
    /// 这是 content 与 trust 之间唯一的受信接线点：content 不直接依赖 trust，
    /// 由应用装配层在这里完成跨域调用，且只做纯追加，不获取新锁。
    /// 管理授权入口。
    ///
    /// DEC-058 已批准以仓库内单一 registry 作为角色与权限的唯一事实源，但 registry 与
    /// `/manage` 属于后续工作包：本轮生产装配里 `management` 为 `None`，任何管理命令
    /// 一律 `PolicyPending` 整体失败关闭。这不是占位，而是显式的未装配态——绝不允许
    /// 「找不到权限规则就放行」。
    fn authorize_management<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        capability: ManagementCapability,
        target: ManagementTarget,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor> {
        Box::pin(async move {
            if !self.management {
                // 授权底座未装配：整体失败关闭，不做任何猜测。
                return Err(ContentError::PolicyPending);
            }
            // 管理动作走独立的管理会话（03 §6），不接受普通会话。
            // 会话令牌与 CSRF 复用同一组凭据字段，但校验路径完全独立。
            let identity = identity::management::authorize_management_session(
                tx,
                &self.renderer,
                credentials.session_token.expose(),
                now,
            )
            .await
            .map_err(failure)?;
            identity
                .check_csrf(credentials.csrf.expose())
                .map_err(failure)?;
            // AUTH-014：未完成 TOTP 绑定前只能访问绑定流程，任何管理动作一律拒绝。
            if identity.access != identity::management::ManagementAccess::Ready {
                return Err(ContentError::Forbidden);
            }
            // 03 §4：授权所需事实必须在最终写入事务内重新读取，不接受事务外快照。
            //
            // `session_authz_version` 直接取账号当前值：管理会话的有效性刚刚校验过，
            // 而 `authz_version` 的作用是让**权限变更**使旧会话失效——变更会同时递增
            // 账号版本并撤销会话，因此这里读到的就是本次会话对应的版本。
            let key = permission_key(capability);
            let version = manage::iam::repo::account_authz(tx, &self.renderer, identity.user_id)
                .await
                .map_err(|_| ContentError::Unavailable)?
                .map(|(_, version)| version)
                .unwrap_or_default();
            let facts = manage::iam::repo::collect_facts(
                tx,
                &self.renderer,
                identity.user_id,
                true,
                true,
                version,
                true,
                key,
                "content",
                target.resource_id,
                now,
            )
            .await
            .map_err(|_| ContentError::Unavailable)?;
            if manage::iam::authorize::authorize(
                &manage::iam::authorize::AuthorizationRequest {
                    actor_id: identity.user_id,
                    permission_key: key,
                    resource_type: "content",
                    resource_id: target.resource_id,
                    // ABAC 版块范围按目标当前所属版块判定（03 §5）。
                    forum_id: target.forum_id,
                },
                &facts,
                now,
            ) != manage::iam::authorize::Decision::Allowed
            {
                return Err(ContentError::Forbidden);
            }
            Ok(ManagementActor {
                user_id: identity.user_id,
            })
        })
    }

    /// 把移帖审计事实落到平台审计表。
    ///
    /// 内容领域不依赖审计存储，由这里完成跨域接线。审计详情只记录不敏感的结构化事实
    /// （前后版块 ID），不写入标题、正文或任何用户可控文本。
    fn append_move_audit<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        actor: &'a ManagementActor,
        topic_id: i64,
        from_forum_id: i64,
        to_forum_id: i64,
        operation_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            use platform::audit::{
                AuditAction, AuditDetails, AuditEvent, AuditReason, AuditResult, AuditSummary,
                AuditTarget, Classification, TargetKind,
            };
            let event = AuditEvent {
                event_id: platform::transaction::new_operation_id(),
                actor_id: actor.user_id,
                action: AuditAction::ContentMoved,
                target: AuditTarget {
                    kind: TargetKind::Topic,
                    id: topic_id.to_string(),
                },
                reason: AuditReason::Moderation,
                ticket_id: None,
                occurred_at: platform::db::encode_time(now),
                result: AuditResult::Succeeded,
                operation_id: operation_id.to_owned(),
                trace_id: None,
                version: 1,
                details: AuditDetails {
                    approval_ids: Vec::new(),
                    // 版块 ID 是 C0 级非敏感结构化事实，按摘要形式登记；
                    // 审计表按设计只接受摘要或 redacted 标志，不存任意原文。
                    before: Some(AuditSummary::from_value(
                        Classification::C0,
                        from_forum_id.to_string().as_bytes(),
                    )),
                    after: Some(AuditSummary::from_value(
                        Classification::C0,
                        to_forum_id.to_string().as_bytes(),
                    )),
                },
                correction_of: None,
            };
            platform_db::audit_repo::append_audit(tx, &self.renderer, &event)
                .await
                .map_err(|_| ContentError::Unavailable)
        })
    }

    fn record_contribution<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        user_id: i64,
        contribution: Contribution,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            match contribution {
                Contribution::Recorded {
                    post_id,
                    is_reply,
                    body_chars,
                } => trust::core::record_contribution(
                    tx,
                    &self.renderer,
                    user_id,
                    if is_reply {
                        trust::contract::ContributionKind::Reply
                    } else {
                        trust::contract::ContributionKind::Topic
                    },
                    post_id,
                    body_chars,
                    now,
                )
                .await
                .map_err(|_| ContentError::Unavailable),
                Contribution::Withdrawn { post_id } => {
                    trust::core::withdraw_contribution(tx, &self.renderer, post_id, now)
                        .await
                        .map_err(|_| ContentError::Unavailable)
                }
            }
        })
    }
    fn forum_rules<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<(i64, String)>> {
        Box::pin(async move {
            moderation::core::content_forum_rules(tx, &self.renderer, ids)
                .await
                .map_err(Into::into)
        })
    }
    fn read_viewer<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        session: Option<&'a str>,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Viewer> {
        Box::pin(async move {
            let Some(raw) = session else {
                return Ok(Viewer::default());
            };
            let Some(user) = identity::core::read_content_identity(tx, &self.renderer, raw, now)
                .await
                .map_err(failure)?
            else {
                return Ok(Viewer::default());
            };
            Ok(Viewer {
                user_id: Some(user.user_id()),
                member: user.email_verified(),
                // DEC-054：被分析排除的账号（系统账号、测试账号、已确认作弊流量）
                // 不计入浏览量，这里把该事实带给内容领域，避免它反查身份仓储。
                excluded: user.analytics_excluded(),
            })
        })
    }
    fn authorize<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        action: Action,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, AuthorityFacts> {
        Box::pin(async move {
            let user = identity::core::authorize_content_identity(
                tx,
                &self.renderer,
                credentials.session_token.expose(),
                credentials.csrf.expose(),
                now,
            )
            .await
            .map_err(failure)?;
            if let Some(context) = credentials.trusted_context.as_ref() {
                let context = context
                    .downcast_ref::<TrustedContentRisk>()
                    .ok_or(ContentError::Forbidden)?;
                context.binding(user.user_id(), action)?;
                if self.risk_rule_version != Some(context.rule_version()) {
                    return Err(ContentError::PolicyPending);
                }
                // This observation distinguishes disabled protection from an allowed bucket.
                match context.protection() {
                    RateProtection::Protected => {
                        tracing::debug!(protection = "protected", "content rate preflight")
                    }
                    RateProtection::NoProtection => {
                        tracing::debug!(protection = "disabled", "content rate preflight")
                    }
                }
            } else if self.risk_rule_version.is_none() {
                return Err(ContentError::PolicyPending);
            }
            let restricted = moderation::core::content_restricted(
                tx,
                &self.renderer,
                user.user_id(),
                capability(action),
                now,
            )
            .await?;
            // DEC-054 确定性 T1 规则的评估点。
            //
            // 锁序：`authorize_content_identity` 已经取过 `trust.identity_snapshots` 行锁，
            // 而 community 各表锁在之后才获取，因此在这里应用授级符合 DEC-052 约定的锁序。
            //
            // 语义：存在生效处罚时不予升级（`restricted` 已经包含账号级 content 限制）。
            // 规则本身与版本号留在 trust 领域，本处只负责注入跨域事实并使用评估结果。
            let evaluated = trust::core::evaluate_tier(
                tx,
                &self.renderer,
                user.user_id(),
                trust::contract::TierInputs {
                    active_penalty: restricted,
                },
                now,
            )
            .await
            .map_err(|_| ContentError::Unavailable)?;
            let snapshot = &evaluated;
            let tier = match snapshot.level() {
                trust::contract::TrustLevel::Pending => TrustTier::Pending,
                trust::contract::TrustLevel::T0 => TrustTier::T0,
                trust::contract::TrustLevel::T1 => TrustTier::T1,
            };
            // No privilege is inferred from invitation eligibility or verification. Tier
            // permissions are evaluated separately by the content policy on these facts.
            Ok(AuthorityFacts {
                user_id: user.user_id(),
                created_at: user.created_at(),
                tier,
                email_verified: user.email_verified(),
                allowed: !restricted,
                default_visibility: self.author_default.ok_or(ContentError::PolicyPending)?,
                publication: Publication::Published,
                analytics_pseudonym: user.analytics_pseudonym().into(),
                analytics_exclusions: TrustedExclusions::from_authority(
                    false,
                    user.analytics_excluded(),
                    false,
                    false,
                ),
            })
        })
    }
    fn authors<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<Author>> {
        Box::pin(async move {
            // 账号级默认值必须已装配；缺失时整体失败关闭，绝不臆造一个公开事实。
            let default_visibility = self.author_default.ok_or(ContentError::PolicyPending)?;
            // 逐用户开关优先于账号级默认：用户关闭后取 Members，未关闭时沿用批准的默认值。
            let axis = |allowed: bool| {
                if allowed {
                    default_visibility
                } else {
                    Visibility::Members
                }
            };
            identity::core::content_authors(tx, &self.renderer, ids)
                .await
                .map_err(failure)
                .map(|authors| {
                    authors
                        .into_iter()
                        .map(|a| Author {
                            user_id: a.user_id,
                            public_id: a.public_ref,
                            username: a.username,
                            body_visibility: axis(a.topic_body_public),
                            lists_visibility: axis(a.profile_lists_public),
                        })
                        .collect()
                })
        })
    }
    fn resolve_author<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        public_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Option<Author>> {
        Box::pin(async move {
            let Some(id) = identity::core::content_profile_id(tx, &self.renderer, public_id)
                .await
                .map_err(failure)?
            else {
                return Ok(None);
            };
            Ok(self.authors(tx, &[id], now).await?.into_iter().next())
        })
    }
    fn append_risk<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        facts: &'a AuthorityFacts,
        action: Action,
        disposition: RiskDisposition,
        event_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            use trust::risk::{
                RiskAction as R, RiskDecision, RiskEventKind as K, RiskReason as Why,
            };
            let signals = credentials
                .trusted_context
                .as_ref()
                .and_then(|v| v.downcast_ref::<TrustedContentRisk>())
                .map(|v| v.signals().clone())
                .unwrap_or_default();
            let decision = match self.risk_rule_version {
                Some(version) => RiskDecision::new(
                    version,
                    match disposition {
                        RiskDisposition::Allowed => R::Allow,
                        RiskDisposition::Denied => R::Deny,
                        RiskDisposition::Unavailable => R::Unavailable,
                    },
                    if disposition == RiskDisposition::Denied {
                        Why::ExplicitDeny
                    } else {
                        Why::ApprovedPolicy
                    },
                ),
                None => RiskDecision::new(0, R::Unavailable, Why::PolicyNotConfigured),
            }
            .map_err(|_| ContentError::Unavailable)?;
            let kind = match action {
                Action::CreateTopic => K::Topic,
                Action::CreateReply => K::Reply,
                Action::EditPost => K::Edit,
                Action::DeletePost => K::Delete,
            };
            trust::core::record_content_decision(
                tx,
                &self.renderer,
                event_id,
                facts.user_id,
                kind,
                signals,
                decision,
                now,
            )
            .await
            .map_err(|_| ContentError::Unavailable)?;
            if !decision.allowed() && disposition == RiskDisposition::Allowed {
                return Err(ContentError::PolicyPending);
            }
            Ok(())
        })
    }
}

pub async fn connect(state: &mut crate::app_state::AppState) -> Result<(), String> {
    state.content.flags = state.identity.flags.clone();
    let Some(key) = &state.config.content.cursor_key else {
        return Ok(());
    };
    let bytes = key
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|p| u8::from_str_radix(p, 16).ok())
                .ok_or("invalid content cursor key")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cursor =
        content::cursor::CursorKey::new(bytes).map_err(|_| "invalid content cursor key")?;
    let renderer = SchemaRenderer::new(&state.config.database.schema_prefix)
        .map_err(|_| "content renderer unavailable")?;
    let pool = platform_db::Pool::connect_existing(&state.config.database)
        .await
        .map_err(|_| "content database unavailable")?;
    let policy = content::policy::ApprovedContentPolicy::new()
        .map_err(|_| "approved content rules are inconsistent")?;
    let mut service = content::core::ContentService::new(
        pool,
        renderer.clone(),
        Arc::new(ApplicationContentAuthority::approved(renderer)),
        Arc::new(policy),
        cursor,
    );
    if let Some(runtime) = state.runtime.as_ref() {
        service = service.with_cache(runtime.cache.clone());
    }
    state.content.service = Some(Arc::new(service));
    if let (Some(key), Some(runtime)) = (&state.config.identity.risk_key, state.runtime.as_ref()) {
        state.content.request_context = Some(Arc::new(
            ContentRiskFactory::new(
                key.as_bytes().to_vec(),
                state.config.features.redis,
                runtime.rate_limiter.clone(),
                Some(
                    crate::content_risk::ContentRatePolicy::approved()
                        .map_err(|_| "approved content rate budget is inconsistent")?,
                ),
                state.config.server.environment.is_production_like(),
            )
            .map_err(|_| "content risk assembly unavailable")?,
        ));
    }
    Ok(())
}
