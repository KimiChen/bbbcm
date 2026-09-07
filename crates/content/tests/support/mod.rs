//! `postgres_concurrency` 与 `postgres_performance` 共用的夹具零件。
//!
//! 抽出来是因为两者的端点闸门、配置构造与合成授权必须**逐字一致**：
//! 一旦各写一份，某天只改了其中一份，两套结果就不再可比，而这种偏差
//! 不会以编译错误的形式暴露。
use chrono::{DateTime, Utc};
use content::contract::*;
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::db::Backend;
use platform::transaction::new_operation_id;
use platform_db::transaction::TransactionConnection;
use std::path::{Path, PathBuf};

/// 仓库内迁移目录。两套测试必须指向同一处，否则「同一批迁移」就不成立。
pub fn migrations_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations")
}

pub fn at() -> DateTime<Utc> {
    Utc::now()
}

pub fn endpoint(var: &str, prefix: &str) -> String {
    assert_eq!(
        std::env::var("POSTGRES_CONTRACT_ISOLATED").ok().as_deref(),
        Some("yes"),
        "必须显式确认目标库是隔离且可丢弃的"
    );
    let raw = std::env::var(var).unwrap_or_else(|_| panic!("需要 {var}"));
    let raw = raw.replace("{prefix}", prefix);
    let parsed = url::Url::parse(&raw).unwrap_or_else(|_| panic!("{var} 的 URL 非法"));
    assert!(
        parsed.host_str().is_some_and(|host| host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())),
        "{var} 只允许字面回环 IP"
    );
    raw
}

pub fn config(prefix: &str, url: &str, max_connections: u32) -> DatabaseConfig {
    let mut cfg = Config::default().database;
    cfg.backend = Backend::Postgres;
    cfg.schema_prefix = prefix.to_owned();
    cfg.postgres_url = Some(SecretString::new(url.to_owned()));
    cfg.max_connections = max_connections;
    cfg
}

/// 本测试要压的是**楼层分配**，不是授权。这个实现一律放行，把授权变量固定下来，
/// 让失败只可能来自并发本身。它绝不能出现在非测试代码里：`authorize` 无条件返回
/// `allowed: true`，用在生产上等于没有权限系统。管理路径仍保持失败关闭，
/// 免得这个放行实现被顺手接到管理动作上。
pub struct AllowAll {
    default_user: i64,
    /// 每个账号一个稳定的分析化名。必须是合法的操作 ID（UUIDv7）——
    /// `platform::analytics` 会校验它的格式，随手拼个 `pseudo-3` 会让
    /// 整条写路径在构造分析事实时失败，且失败被归一成 `Unavailable`。
    pseudonyms: std::collections::BTreeMap<i64, String>,
}

impl AllowAll {
    /// `users` 至少一项；第一项作为默认访问者。
    pub fn new(users: Vec<i64>) -> Self {
        let default_user = users[0];
        Self {
            default_user,
            pseudonyms: users
                .into_iter()
                .map(|id| (id, new_operation_id()))
                .collect(),
        }
    }

    fn facts(&self, user_id: i64) -> AuthorityFacts {
        AuthorityFacts {
            user_id,
            // 账龄取足够久，确保 T1 规则不因「账号太新」而拒绝。
            created_at: at() - chrono::Duration::days(30),
            tier: TrustTier::T1,
            email_verified: true,
            allowed: true,
            default_visibility: Visibility::Public,
            publication: Publication::Published,
            analytics_pseudonym: self
                .pseudonyms
                .get(&user_id)
                .cloned()
                .unwrap_or_else(new_operation_id),
            analytics_exclusions: platform::analytics::TrustedExclusions::from_authority(
                false, true, false, false,
            ),
        }
    }
}

impl ContentAuthority for AllowAll {
    fn read_viewer<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _session: Option<&'a str>,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Viewer> {
        // 必须按 session 判定成员身份。恒返回 `member: true` 会让所有「匿名不可见」
        // 的断言变成空断言——过滤条件里 `member = 1` 一路放行，测什么都通过。
        let member = _session.is_some();
        Box::pin(async move {
            Ok(Viewer {
                user_id: member.then_some(self.default_user),
                member,
                excluded: true,
            })
        })
    }

    fn authorize<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        _action: Action,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, AuthorityFacts> {
        // 夹具把用户 ID 直接放进 session token；这是测试自造的凭据，
        // 不经过任何真实会话解析，也不代表生产可以这样取身份。
        let user_id: i64 = credentials
            .session_token
            .expose()
            .parse()
            .unwrap_or(self.default_user);
        Box::pin(async move { Ok(self.facts(user_id)) })
    }

    fn resolve_author<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _public_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Option<Author>> {
        Box::pin(async { Ok(None) })
    }

    fn authors<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        user_ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<Author>> {
        Box::pin(async move {
            Ok(user_ids
                .iter()
                .map(|id| Author {
                    user_id: *id,
                    public_id: format!("pub-{id}"),
                    username: format!("u{id}"),
                    body_visibility: Visibility::Public,
                    lists_visibility: Visibility::Public,
                })
                .collect())
        })
    }

    fn authorize_management<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _credentials: &'a Credentials,
        _capability: ManagementCapability,
        _target: ManagementTarget,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor> {
        Box::pin(async { Err(ContentError::PolicyPending) })
    }

    fn append_move_audit<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _actor: &'a ManagementActor,
        _topic_id: i64,
        _from_forum_id: i64,
        _to_forum_id: i64,
        _operation_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn record_contribution<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _user_id: i64,
        _contribution: Contribution,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn append_risk<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _credentials: &'a Credentials,
        _facts: &'a AuthorityFacts,
        _action: Action,
        _disposition: RiskDisposition,
        _event_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
