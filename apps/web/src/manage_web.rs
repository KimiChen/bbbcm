//! `/manage` 控制台的装配：模板渲染与后端接线。
//!
//! 所有授权判定都下沉到既有 `management_gate`，本模块不做第二份判定；
//! 它只负责「把表单变成命令调用」与「把领域结果变成视图模型」。
use crate::{
    app_state::AppState,
    assets::{Assets, filters},
};
use askama::Template;
use chrono::Utc;
use manage::console::{
    http::{BoxFuture, ConsoleBackend, ConsoleError, ConsoleState},
    view::{CaseDetail, CaseRow, Link, Page, PageRenderer, RenderError},
};
use platform::identifiers::SchemaRenderer;
use platform_db::{Pool, transaction::DbTransaction};
use std::{collections::HashMap, sync::Arc};

/// `layout/base.html` 里 `error_block` 需要的最小外壳。
///
/// 管理后台自己渲染错误与提示（`page.error`／`page.notice`），因此三项恒为空——
/// 保留结构只是为了复用同一套已冻结的基础模板。
struct Frame {
    flash: Option<String>,
    error: Option<String>,
    empty: Option<String>,
}

#[derive(Template)]
#[template(path = "manage/console.html")]
struct ConsoleTemplate<'a> {
    page: &'a Page,
    frame: &'a Frame,
}

struct Renderer(Assets);
impl PageRenderer for Renderer {
    fn render(&self, page: &Page) -> Result<String, RenderError> {
        ConsoleTemplate {
            page,
            frame: &Frame {
                flash: None,
                error: None,
                empty: None,
            },
        }
        .render_with_values(&("assets", &self.0 as &dyn std::any::Any))
        .map_err(|_| RenderError)
    }
}

/// 未装配数据库时的控制台状态：后端为 `None`，所有管理路由直接 404。
pub fn closed(config: &platform::config::Config, assets: &Assets) -> ConsoleState {
    ConsoleState {
        backend: None,
        renderer: Arc::new(Renderer(assets.clone())),
        secure_cookies: config.server.environment.is_production_like(),
    }
}

/// 装配真实后端。
///
/// 与内容/治理授权在同一处装配，避免三者的装配状态出现分叉（一边开着一边关着）。
/// MFA keyring 未注入时 `mfa` 为 `None`，绑定与验签一律失败关闭（`DEC-062`）。
pub async fn connect(state: &mut AppState) {
    let (Some(service), Ok(renderer)) = (
        state.content.service.clone(),
        SchemaRenderer::new(&state.config.database.schema_prefix),
    ) else {
        return;
    };
    let Ok(pool) = platform_db::Pool::connect_existing(&state.config.database).await else {
        return;
    };
    state.console.backend = Some(Arc::new(Backend {
        pool,
        renderer,
        mfa: state
            .config
            .identity
            .mfa_key
            .as_ref()
            .map(|key| key.as_bytes().to_vec()),
        content: service,
        identity: state.identity.service.clone(),
    }));
}

/// 真实后端。持有连接池与 schema 渲染器，每个动作各自开事务。
struct Backend {
    pool: Pool,
    renderer: SchemaRenderer,
    /// MFA keyring 材料（`DEC-062`）。未注入时绑定与验签一律失败关闭。
    mfa: Option<Vec<u8>>,
    /// 内容服务。移帖命令自带事务、锁序、审计与授权，控制台直接调用它。
    content: Arc<content::core::ContentService>,
    /// 身份服务。管理登录复用它的密码校验与限流路径，不另建一条。
    identity: Option<Arc<identity::core::IdentityService>>,
}

fn status_label(status: moderation::cases::CaseStatus) -> &'static str {
    use moderation::cases::CaseStatus::*;
    match status {
        Triage => "待分流",
        Investigating => "调查中",
        AwaitingInfo => "等待补充",
        Resolved => "已处理",
        Appealed => "已申诉",
        Closed => "已关闭",
    }
}
fn priority_label(priority: i64) -> &'static str {
    match priority {
        1 => "高",
        2 => "中",
        _ => "低",
    }
}

impl ConsoleBackend for Backend {
    fn session<'a>(&'a self, raw: &'a str) -> BoxFuture<'a, Result<(String, bool), ConsoleError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let result = identity::management::authorize_management_session(
                &mut tx.connection(),
                &self.renderer,
                raw,
                Utc::now(),
            )
            .await;
            // 会话校验会刷新空闲计时，因此必须提交；失败则回滚。
            let outcome = match result {
                Ok(identity) => {
                    let bound = identity.access == identity::management::ManagementAccess::Ready;
                    // 显示名只用账号 ID 的公开形态，不带邮箱等 C2 资料。
                    Ok((format!("操作者 #{}", identity.user_id), bound))
                }
                Err(_) => Err(ConsoleError::Unauthenticated),
            };
            if outcome.is_ok() {
                tx.commit().await.map_err(|_| ConsoleError::Unavailable)?;
            } else {
                let _ = tx.rollback().await;
            }
            outcome
        })
    }

    fn login<'a>(
        &'a self,
        username: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<(String, String, bool), ConsoleError>> {
        Box::pin(async move {
            let identity = self.identity.as_ref().ok_or(ConsoleError::Unavailable)?;
            // 复用与普通登录完全相同的请求上下文构造：四维限流、风险信号与挑战
            // 都在这一层生效，管理登录不走任何简化路径。
            let context = identity::contract::RequestContext::new(
                identity::contract::Secret::from(platform::transaction::new_operation_id()),
                identity::contract::Secret::from(platform::transaction::new_operation_id()),
                platform::transaction::new_operation_id(),
            );
            identity
                .management_login(
                    &context,
                    identity::contract::LoginInput {
                        identifier: username.to_owned(),
                        password: identity::contract::Secret::from(password.to_owned()),
                        device_summary: "unknown".into(),
                    },
                    Utc::now(),
                )
                .await
                .map(|result| {
                    (
                        result.token.expose().to_owned(),
                        result.csrf.expose().to_owned(),
                        result.bound,
                    )
                })
                // 账号不存在、不是 staff、密码错误在这里已经收敛成同一个错误，
                // 控制台再统一成一句「登录失败」，不给任何区分线索。
                .map_err(|_| ConsoleError::Unauthenticated)
        })
    }

    fn begin_binding<'a>(
        &'a self,
        raw: &'a str,
    ) -> BoxFuture<'a, Result<(String, String), ConsoleError>> {
        Box::pin(async move {
            let material = self.mfa.as_ref().ok_or(ConsoleError::Unavailable)?;
            let wrapper = identity::mfa::SecretWrapper::new(material, 1)
                .map_err(|_| ConsoleError::Unavailable)?;
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                let identity = identity::management::authorize_management_session(
                    &mut c,
                    &self.renderer,
                    raw,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Unauthenticated)?;
                identity::management::begin_mfa_binding(
                    &mut c,
                    &self.renderer,
                    &wrapper,
                    identity.user_id,
                    &format!("operator-{}", identity.user_id),
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Unavailable)
            }
            .await;
            if outcome.is_ok() {
                tx.commit().await.map_err(|_| ConsoleError::Unavailable)?;
            } else {
                let _ = tx.rollback().await;
            }
            outcome
        })
    }

    fn confirm_binding<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
        code: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, ConsoleError>> {
        Box::pin(async move {
            let material = self.mfa.as_ref().ok_or(ConsoleError::Unavailable)?;
            let wrapper = identity::mfa::SecretWrapper::new(material, 1)
                .map_err(|_| ConsoleError::Unavailable)?;
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                let identity = identity::management::authorize_management_session(
                    &mut c,
                    &self.renderer,
                    raw,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Unauthenticated)?;
                identity
                    .check_csrf(csrf)
                    .map_err(|_| ConsoleError::Denied)?;
                identity::management::confirm_mfa_binding(
                    &mut c,
                    &self.renderer,
                    &wrapper,
                    &identity,
                    code,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::InvalidInput)
            }
            .await;
            if outcome.is_ok() {
                tx.commit().await.map_err(|_| ConsoleError::Unavailable)?;
            } else {
                let _ = tx.rollback().await;
            }
            outcome
        })
    }

    fn cases<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
    ) -> BoxFuture<'a, Result<Vec<CaseRow>, ConsoleError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                // 读队列同样要过授权门：没有 cases.read 权限的角色看不到队列。
                crate::management_gate::authorize(
                    &mut c,
                    &self.renderer,
                    raw,
                    csrf,
                    "moderation.cases.read",
                    "case",
                    0,
                    None,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Denied)?;
                moderation::case_repo::queue(&mut c, &self.renderer, 50)
                    .await
                    .map_err(|_| ConsoleError::Unavailable)
            }
            .await;
            let _ = tx.rollback().await;
            outcome.map(|rows| {
                rows.into_iter()
                    .map(|case| CaseRow {
                        id: case.id,
                        href: format!("/manage/cases/{}", case.id),
                        target: format!("{} #{}", case.target_kind, case.target_id),
                        status: status_label(case.status),
                        priority: priority_label(case.priority),
                        escalated: case.escalated,
                        reports: case.report_count,
                    })
                    .collect()
            })
        })
    }

    fn case<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
        id: i64,
    ) -> BoxFuture<'a, Result<CaseDetail, ConsoleError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                crate::management_gate::authorize(
                    &mut c,
                    &self.renderer,
                    raw,
                    csrf,
                    "moderation.cases.read",
                    "case",
                    id,
                    None,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Denied)?;
                let case = moderation::case_repo::case(&mut c, &self.renderer, id)
                    .await
                    .map_err(|_| ConsoleError::Unavailable)?
                    .ok_or(ConsoleError::InvalidInput)?;
                // 只取摘要与时间，不取正文：证据正文属 C2，`/manage` 列表与详情
                // 都不返回它，需要时另走带敏感访问审计的授权读取路径。
                let evidence = moderation::case_repo::evidence_digests(&mut c, &self.renderer, id)
                    .await
                    .map_err(|_| ConsoleError::Unavailable)?;
                Ok::<_, ConsoleError>(CaseDetail {
                    id: case.id,
                    target: format!("案件 #{}", case.id),
                    status: status_label(case.status),
                    priority: priority_label(case.priority),
                    escalated: case.escalated,
                    reports: case.report_count,
                    evidence: evidence
                        .into_iter()
                        .map(|(digest, at)| (digest, at.format("%Y-%m-%d %H:%M UTC").to_string()))
                        .collect(),
                    // 可推进的状态由封闭迁移表算出，前端不做判断。
                    transitions: allowed_transitions(case.status, id),
                    report_details: moderation::case_repo::report_details(
                        &mut c,
                        &self.renderer,
                        id,
                        100,
                    )
                    .await
                    .map_err(|_| ConsoleError::Unavailable)?
                    .into_iter()
                    .map(|(category, at, reporter)| {
                        (
                            category_label(&category).to_owned(),
                            at.format("%Y-%m-%d %H:%M UTC").to_string(),
                            format!("#{reporter}"),
                        )
                    })
                    .collect(),
                })
            }
            .await;
            let _ = tx.rollback().await;
            outcome
        })
    }

    fn case_action<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
        id: i64,
        form: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<(), ConsoleError>> {
        Box::pin(async move {
            let authority = moderation_authority_for(&self.renderer);
            let credentials =
                moderation::authority::ManagementCredentials::new(raw.to_owned(), csrf.to_owned());
            let target = moderation::authority::ManagementTarget {
                resource_id: id,
                forum_id: None,
            };
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                if let Some(note) = form.get("note").filter(|v| !v.trim().is_empty()) {
                    moderation::cases::add_note(
                        &mut c,
                        &self.renderer,
                        &authority,
                        &credentials,
                        target,
                        id,
                        note,
                        Utc::now(),
                    )
                    .await
                    .map_err(map_moderation)?;
                }
                if let Some(next) = form.get("status") {
                    let next = moderation::cases::CaseStatus::parse(next)
                        .map_err(|_| ConsoleError::InvalidInput)?;
                    moderation::cases::advance_status(
                        &mut c,
                        &self.renderer,
                        &authority,
                        &credentials,
                        target,
                        id,
                        next,
                        Utc::now(),
                    )
                    .await
                    .map_err(map_moderation)?;
                }
                Ok::<(), ConsoleError>(())
            }
            .await;
            if outcome.is_ok() {
                tx.commit().await.map_err(|_| ConsoleError::Unavailable)?;
            } else {
                let _ = tx.rollback().await;
            }
            outcome
        })
    }

    fn move_form<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
        topic_id: i64,
    ) -> BoxFuture<'a, Result<manage::console::http::MoveForm, ConsoleError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                // 加载表单同样要过授权门：没有移帖权限的人连表单都不该看到，
                // 否则它会变成一个「这个主题在哪个版块」的探测接口。
                crate::management_gate::authorize(
                    &mut c,
                    &self.renderer,
                    raw,
                    csrf,
                    "community.content.move",
                    "content",
                    topic_id,
                    None,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Denied)?;
                let catalog = self
                    .content
                    .catalog(Utc::now())
                    .await
                    .map_err(|_| ConsoleError::Unavailable)?;
                let topic = self
                    .content
                    .topic_summary(topic_id, Utc::now())
                    .await
                    .map_err(|_| ConsoleError::InvalidInput)?;
                let current = catalog
                    .iter()
                    .find(|f| f.id == topic.1)
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| format!("版块 #{}", topic.1));
                // 目标版块列表排除当前版块：移到原地会被命令拒绝，
                // 让它出现在下拉里只会制造一次必然失败的操作。
                let forums = catalog
                    .into_iter()
                    .filter(|f| f.id != topic.1)
                    .map(|f| (f.id.to_string(), f.name))
                    .collect();
                Ok::<_, ConsoleError>((topic.0, current, topic.2, forums))
            }
            .await;
            let _ = tx.rollback().await;
            outcome
        })
    }

    fn reports<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
    ) -> BoxFuture<'a, Result<Vec<manage::console::http::ReportRow>, ConsoleError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ConsoleError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                crate::management_gate::authorize(
                    &mut c,
                    &self.renderer,
                    raw,
                    csrf,
                    "moderation.cases.read",
                    "case",
                    0,
                    None,
                    Utc::now(),
                )
                .await
                .map_err(|_| ConsoleError::Denied)?;
                moderation::case_repo::report_details(&mut c, &self.renderer, 0, 100)
                    .await
                    .map_err(|_| ConsoleError::Unavailable)
            }
            .await;
            let _ = tx.rollback().await;
            outcome.map(|rows| {
                rows.into_iter()
                    .map(|(category, at, reporter)| {
                        (
                            category_label(&category).to_owned(),
                            at.format("%Y-%m-%d %H:%M UTC").to_string(),
                            // 只给举报人的内部 ID，不带用户名或邮箱：
                            // 举报人身份属 C2，暴露它会让举报变成一件有风险的事。
                            format!("#{reporter}"),
                        )
                    })
                    .collect()
            })
        })
    }

    fn move_topic<'a>(
        &'a self,
        raw: &'a str,
        csrf: &'a str,
        form: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<i64, ConsoleError>> {
        Box::pin(async move {
            let number = |key: &str| -> Result<i64, ConsoleError> {
                form.get(key)
                    .and_then(|v| v.trim().parse().ok())
                    .filter(|v: &i64| *v > 0)
                    .ok_or(ConsoleError::InvalidInput)
            };
            let (topic, forum, version) =
                (number("topic_id")?, number("forum_id")?, number("version")?);
            let credentials = content::contract::Credentials::new(
                raw.to_owned(),
                csrf.to_owned(),
                platform::transaction::new_operation_id(),
            );
            // move_topic 自带完整事务、锁序、审计与授权，这里不再包一层事务。
            self.content
                .move_topic(&credentials, topic, forum, version, Utc::now())
                .await
                .map(|result| result.topic_id)
                .map_err(|error| match error {
                    content::contract::ContentError::Forbidden
                    | content::contract::ContentError::PolicyPending => ConsoleError::Denied,
                    content::contract::ContentError::InvalidInput
                    | content::contract::ContentError::Conflict
                    | content::contract::ContentError::NotFound => ConsoleError::InvalidInput,
                    _ => ConsoleError::Unavailable,
                })
        })
    }
}

/// 举报类型的中文标签。未知取值原样返回，不猜测也不隐藏。
fn category_label(key: &str) -> &str {
    match key {
        "spam" => "垃圾广告",
        "harassment" => "辱骂骚扰",
        "illegal" => "违法违规",
        "privacy" => "隐私泄露",
        "infringement" => "侵权",
        "fraud" => "诈骗",
        "malware" => "恶意文件",
        "other" => "其他",
        other => other,
    }
}

fn map_moderation(error: moderation::contract::ModerationError) -> ConsoleError {
    use moderation::contract::ModerationError as E;
    match error {
        E::Denied => ConsoleError::Denied,
        E::InvalidInput | E::NotFound | E::Conflict => ConsoleError::InvalidInput,
        E::Unavailable => ConsoleError::Unavailable,
    }
}

fn moderation_authority_for(
    renderer: &SchemaRenderer,
) -> crate::moderation_authority::ApplicationModerationAuthority {
    crate::moderation_authority::ApplicationModerationAuthority::approved(renderer.clone())
}

/// 由封闭迁移表算出可推进的状态，供页面渲染动作链接。
fn allowed_transitions(current: moderation::cases::CaseStatus, id: i64) -> Vec<Link> {
    use moderation::cases::CaseStatus::*;
    [
        (Investigating, "开始调查"),
        (AwaitingInfo, "等待补充"),
        (Resolved, "标记已处理"),
        (Closed, "关闭"),
    ]
    .into_iter()
    .filter(|(next, _)| current.permits(*next))
    .map(|(next, label)| {
        Link::new(
            label,
            format!("/manage/cases/{id}?status={}", next.as_str()),
        )
    })
    .collect()
}
