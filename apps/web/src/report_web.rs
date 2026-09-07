//! 举报入口的装配：模板渲染与后端接线（`MOD-001`／`MOD-002`）。
//!
//! 与 `manage_web` 同一范式——领域 crate 只给视图模型与 trait，模板与身份接线
//! 都在这里。会话与 CSRF 判定复用 `identity::content_access` 的同一条路径，
//! 本模块不做第二份判定：举报是普通用户的写操作，和发帖走同一套资格与 CSRF 规则。
use crate::{
    app_state::AppState,
    assets::{Assets, filters},
};
use askama::Template;
use chrono::Utc;
use moderation::http::{
    RenderError, ReportBackend, ReportError, ReportPage, ReportRenderer, ReportState,
};
use platform::identifiers::SchemaRenderer;
use platform_db::{Pool, transaction::DbTransaction};
use std::{future::Future, pin::Pin, sync::Arc};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct Frame {
    flash: Option<String>,
    error: Option<String>,
    empty: Option<String>,
}

#[derive(Template)]
#[template(path = "moderation/report.html")]
struct ReportTemplate<'a> {
    page: &'a ReportPage,
    frame: &'a Frame,
}

struct Renderer(Assets);
impl ReportRenderer for Renderer {
    fn render(&self, page: &ReportPage) -> Result<String, RenderError> {
        ReportTemplate {
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

/// 未装配数据库时返回 `None`：整组举报路由不注册，由框架 404。
///
/// 不注册而不是注册后回 503：一个稳定回 503 的入口同样能被用来探测部署状态，
/// 而 404 与「本来就没有这个路径」不可区分。
pub async fn connect(state: &AppState) -> Option<ReportState> {
    let renderer = SchemaRenderer::new(&state.config.database.schema_prefix).ok()?;
    let pool = Pool::connect_existing(&state.config.database).await.ok()?;
    Some(ReportState {
        backend: Arc::new(Backend { pool, renderer }),
        renderer: Arc::new(Renderer(state.assets.clone())),
        secure_cookies: state.config.server.environment.is_production_like(),
    })
}

struct Backend {
    pool: Pool,
    renderer: SchemaRenderer,
}

impl ReportBackend for Backend {
    fn viewer<'a>(&'a self, raw_session: &'a str) -> BoxFuture<'a, Result<(), ReportError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin_read(&self.pool)
                .await
                .map_err(|_| ReportError::Unavailable)?;
            let found = identity::core::read_content_identity(
                &mut tx.connection(),
                &self.renderer,
                raw_session,
                Utc::now(),
            )
            .await;
            // 只读事务无论成败都回滚：不留悬挂事务占住连接。
            let _ = tx.rollback().await;
            match found {
                Ok(Some(_)) => Ok(()),
                Ok(None) => Err(ReportError::Unauthenticated),
                Err(_) => Err(ReportError::Unavailable),
            }
        })
    }

    fn submit<'a>(
        &'a self,
        session: &'a str,
        csrf: &'a str,
        kind: &'a str,
        target_id: i64,
        category: moderation::cases::ReportCategory,
        note: &'a str,
    ) -> BoxFuture<'a, Result<i64, ReportError>> {
        Box::pin(async move {
            let target_kind = moderation::cases::TargetKind::parse(kind)
                .map_err(|_| ReportError::InvalidInput)?;
            let now = Utc::now();
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| ReportError::Unavailable)?;
            let outcome = async {
                let mut c = tx.connection();
                // 与发帖同一条资格判定：会话有效、CSRF 与会话绑定、账号可写。
                // 举报不该比发帖更宽松——那会让被限制发言的账号仍能刷举报。
                let identity = identity::core::authorize_content_identity(
                    &mut c,
                    &self.renderer,
                    session,
                    csrf,
                    now,
                )
                .await
                .map_err(|_| ReportError::Unauthenticated)?;
                let outcome = moderation::cases::submit_report(
                    &mut c,
                    &self.renderer,
                    identity.user_id(),
                    target_kind,
                    target_id,
                    category,
                    note,
                    None,
                    now,
                )
                .await
                .map_err(|_| ReportError::Rejected)?;
                Ok::<i64, ReportError>(outcome.case_id)
            }
            .await;
            match outcome {
                Ok(case) => {
                    tx.commit().await.map_err(|_| ReportError::Unavailable)?;
                    Ok(case)
                }
                Err(err) => {
                    let _ = tx.rollback().await;
                    Err(err)
                }
            }
        })
    }
}
