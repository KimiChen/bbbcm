//! 治理领域路由：举报入口与流程页面。随 WP-MVP-D01 接入。
//!
//! 路由对进程状态类型 `S` 泛型：领域 crate 不依赖 `apps/web`，由 `apps/web/src/router.rs`
//! 在装配时决定 `S` 的具体类型并统一 `with_state`。handler 取子状态用 `State<T>` +
//! `FromRef<S>`，其中 `T` 是 `platform-db` 暴露的事务模板/服务句柄，不是裸连接池——
//! `02` 第 2.1 节规定 handler 不得直接写 SQL。

pub fn router<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    axum::Router::new()
}
