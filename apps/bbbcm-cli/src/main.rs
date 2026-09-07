//! bbbcm-cli：迁移、种子数据、账号引导与 Feature Flag 管理。
//! 命令实现为薄壳：参数解析后调用 crate 服务（迁移在 platform-db，用户创建走 identity）。

use clap::{Parser, Subcommand, ValueEnum};
use platform::db::Backend;

#[derive(Debug, Parser)]
#[command(name = "bbbcm-cli", version, about = "草莓社区运维与引导命令行")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 数据库结构迁移与状态
    Db {
        /// 版本化迁移根目录；不接受 SQL 内联参数
        #[arg(long, global = true, default_value = "db/migrations")]
        migrations_dir: std::path::PathBuf,
        #[command(subcommand)]
        command: DbCommand,
    },
    /// 账号引导与救援
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Feature Flag 查看与修改（默认全部关闭）
    Flag {
        #[command(subcommand)]
        command: FlagCommand,
    },
    /// 管理角色授予与撤销（受控命令，全部记审计）
    Iam {
        #[command(subcommand)]
        command: IamCommand,
    },
}

#[derive(Debug, Subcommand)]
enum IamCommand {
    /// 授予管理角色。`--scope forums` 时必须至少给一个 `--forum`
    Grant {
        /// 被授予人用户名
        username: String,
        /// 角色 key，取 `03` §4 registry 中的值
        role: String,
        /// 授予人用户名。**不得与被授予人相同**——职责分离要求授权由他人执行
        #[arg(long)]
        by: String,
        #[arg(long, default_value = "global")]
        scope: String,
        /// `--scope forums` 的版块 slug，可重复
        #[arg(long)]
        forum: Vec<String>,
    },
    /// 撤销一条角色分配
    Revoke {
        /// `iam.role_assignments` 的行 ID
        assignment_id: i64,
        /// 执行撤销的账号用户名
        #[arg(long)]
        by: String,
    },
    /// 列出某账号当前生效的角色分配
    List { username: String },
}

#[derive(Debug, Subcommand)]
enum DbCommand {
    /// 执行迁移；--backend 缺省时读配置
    Migrate {
        #[arg(long, value_enum)]
        backend: Option<BackendArg>,
    },
    /// 查看迁移与结构版本状态
    Status,
    /// 输出一次性 PostgreSQL 组角色初始化 SQL（需数据库管理员另行执行）
    BootstrapSql,
    /// 写入 development/test 六版块与规则占位（不创建账号）
    Seed,
    /// 应用完整模式搜索索引轨（`postgres-search`）；缺 pg_jieba 时跳过而非失败
    SearchIndex,
}

/// `--backend` 的合法取值。用枚举而非自由字符串：拼错的后端名必须在参数解析阶段
/// 就被拒绝并列出候选，而不是等到连接数据库时才失败。
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
enum BackendArg {
    Postgres,
    Sqlite,
    /// 规范预留，尚未实现（`DEC-022`／`DEC-033`）。
    Mysql8,
}

impl From<BackendArg> for Backend {
    fn from(arg: BackendArg) -> Self {
        match arg {
            BackendArg::Postgres => Backend::Postgres,
            BackendArg::Sqlite => Backend::Sqlite,
            BackendArg::Mysql8 => Backend::Mysql8,
        }
    }
}

#[derive(Debug, Subcommand)]
enum UserCommand {
    /// 创建用户
    Create { username: String },
    /// 提升为管理员
    Promote { username: String },
    /// 重置密码
    ResetPassword { username: String },
}

#[derive(Debug, Subcommand)]
enum FlagCommand {
    /// 从数据库列出当前值、版本、缺项与注册默认值
    List,
    /// 修改 flag（受控命令）
    Set {
        name: String,
        // clap 的 derive 默认把 `bool` 映射成 SetTrue（开关语义），而这是一个
        // **位置**参数，位置参数必须取值，两者冲突。debug 构建里表现为
        // `Cli::parse()` 一进来就 panic——连 `flag set --help` 都打不开；
        // release 构建里 debug_assert 被编译掉，冲突不再报错，取值行为却仍然
        // 不是这里想要的。显式声明 Set 动作，让它按「解析成 true/false」处理。
        #[arg(action = clap::ArgAction::Set)]
        value: bool,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    use platform::config::{Config, ProcessKind};
    let mut config = Config::from_environment().map_err(|err| err.to_string())?;
    config
        .validate_for(ProcessKind::Cli)
        .map_err(|err| err.to_string())?;
    match cli.command {
        Command::Db {
            migrations_dir,
            command,
        } => match command {
            DbCommand::Migrate { backend } => {
                if let Some(backend) = backend {
                    config.database.backend = backend.into();
                }
                config.validate().map_err(|err| err.to_string())?;
                let result = platform_db::migrations::migrate(&config.database, &migrations_dir)
                    .await
                    .map_err(|err| err.to_string())?;
                println!(
                    "迁移完成：应用 {:?}；已应用跳过 {:?}",
                    result.applied, result.skipped
                );
            }
            // 与 Migrate 分开的原因见 `platform_db::search_index` 模块头：
            // `02` §6.5 允许缺 pg_jieba 的实例正常启动、只关搜索，因此索引不能
            // 进常规轨——那会把扩展升格为迁移期硬依赖。
            DbCommand::SearchIndex => {
                config.validate().map_err(|err| err.to_string())?;
                let outcome =
                    platform_db::search_index::migrate_search(&config.database, &migrations_dir)
                        .await
                        .map_err(|err| err.to_string())?;
                match outcome {
                    platform_db::search_index::SearchIndexOutcome::Applied { applied, skipped } => {
                        println!("搜索索引完成：应用 {applied:?}；已应用跳过 {skipped:?}");
                    }
                    // 退出码保持 0：这是 `02` §6.5 允许的降级状态，不是失败。
                    // 但必须打印清楚缺什么，否则运维会以为索引建好了。
                    platform_db::search_index::SearchIndexOutcome::ExtensionMissing { missing } => {
                        println!(
                            "跳过搜索索引：缺少扩展 {missing:?}。该实例按 02 §6.5 以降级模式运行，搜索能力关闭；装上扩展后重跑本命令。"
                        );
                    }
                }
            }
            DbCommand::Status => {
                let states = platform_db::migrations::status(&config.database, &migrations_dir)
                    .await
                    .map_err(|err| err.to_string())?;
                println!(
                    "后端 {}，前缀 {}",
                    config.database.backend, config.database.schema_prefix
                );
                for state in states {
                    println!(
                        "{:04} {} {} {} {}",
                        state.number,
                        state.name,
                        state.state,
                        state.checksum,
                        state.executed_at.as_deref().unwrap_or("-")
                    );
                }
            }
            DbCommand::Seed => {
                if config.server.environment.is_production_like() {
                    return Err(
                        "db seed 只允许 development/test；staging/production 禁止种子写入".into(),
                    );
                }
                let states = platform_db::migrations::status(&config.database, &migrations_dir)
                    .await
                    .map_err(|err| err.to_string())?;
                if states.iter().any(|s| s.state != "applied") {
                    return Err("种子需要所有迁移已应用；请先执行 db migrate".into());
                }
                let result = platform_db::seed::seed(&config)
                    .await
                    .map_err(|err| err.to_string())?;
                println!(
                    "种子完成：新增 {} 个版块、{} 个规则占位版本；已有数据保留",
                    result.forums_created, result.rules_created
                );
            }
            DbCommand::BootstrapSql => {
                let renderer =
                    platform::identifiers::SchemaRenderer::new(&config.database.schema_prefix)
                        .map_err(|err| err.to_string())?;
                print!(
                    "{}",
                    platform_db::acl::bootstrap_sql(&renderer).map_err(|err| err.to_string())?
                );
            }
        },
        Command::User { command } => match command {
            UserCommand::Create { username } => {
                not_implemented(&format!("user create（{username}），随 WP-MVP-B04 接入"));
            }
            // AUTH-014：首个站点所有者的一次性提权。
            //
            // 只有在系统尚无任何 staff principal 时才允许执行；已存在时必须拒绝，
            // 否则这条命令就变成一条常驻的本地提权后门。提权与拒绝都写追加式审计，
            // 并在标准错误上打印告警行，供运维侧采集。
            //
            // 本命令**只**建立 staff principal，不授予任何角色或权限——角色到能力的
            // 映射属 WP-MVP-D04 的权限 registry。提权后的账号首次管理登录会被强制
            // 进入 TOTP 绑定流程，未完成前管理会话只能访问绑定流程。
            UserCommand::Promote { username } => {
                use platform::audit::{
                    AuditAction, AuditDetails, AuditEvent, AuditReason, AuditResult, AuditSummary,
                    AuditTarget, Classification, TargetKind,
                };
                let pool = platform_db::Pool::connect_existing(&config.database)
                    .await
                    .map_err(|err| err.to_string())?;
                let renderer =
                    platform::identifiers::SchemaRenderer::new(&config.database.schema_prefix)
                        .map_err(|err| err.to_string())?;
                let now = chrono::Utc::now();
                let mut tx = platform_db::transaction::DbTransaction::begin(&pool)
                    .await
                    .map_err(|err| err.to_string())?;
                let outcome = async {
                    let mut c = tx.connection();
                    let account_id =
                        identity::management::account_id_by_username(&mut c, &renderer, &username)
                            .await
                            .map_err(|err| err.to_string())?
                            .ok_or_else(|| "账号不存在".to_owned())?;
                    let granted = identity::management::bootstrap_first_owner(
                        &mut c, &renderer, account_id, now,
                    )
                    .await
                    .is_ok();
                    let event = AuditEvent {
                        event_id: platform::transaction::new_operation_id(),
                        actor_id: account_id,
                        action: AuditAction::OwnerBootstrapped,
                        target: AuditTarget {
                            kind: TargetKind::StaffPrincipal,
                            id: account_id.to_string(),
                        },
                        reason: AuditReason::Recovery,
                        ticket_id: None,
                        occurred_at: platform::db::encode_time(now),
                        result: if granted {
                            AuditResult::Succeeded
                        } else {
                            AuditResult::Denied
                        },
                        operation_id: platform::transaction::new_operation_id(),
                        trace_id: None,
                        version: 1,
                        details: AuditDetails {
                            approval_ids: Vec::new(),
                            before: None,
                            after: Some(AuditSummary::from_value(
                                Classification::C0,
                                account_id.to_string().as_bytes(),
                            )),
                        },
                        correction_of: None,
                    };
                    platform_db::audit_repo::append_audit(&mut c, &renderer, &event)
                        .await
                        .map_err(|err| err.to_string())?;
                    Ok::<bool, String>(granted)
                }
                .await;
                match outcome {
                    Ok(granted) => {
                        tx.commit().await.map_err(|err| err.to_string())?;
                        // 无论成功与否都告警：一次成功的首账号提权与一次被拒的尝试，
                        // 都是需要人工复盘的高危事件。
                        eprintln!(
                            "[ALERT] AUTH-014 首账号提权 {}：{username}",
                            if granted { "已执行" } else { "被拒绝" }
                        );
                        if !granted {
                            return Err("系统已存在特权账号，提权路径已关闭".to_owned());
                        }
                        println!("已创建首个站点所有者的 staff principal：{username}");
                        println!("首次管理登录将强制绑定 TOTP，未完成前只能访问绑定流程。");
                    }
                    Err(err) => {
                        tx.rollback().await.map_err(|e| e.to_string())?;
                        return Err(err);
                    }
                }
            }
            UserCommand::ResetPassword { username } => {
                not_implemented(&format!(
                    "user reset-password（{username}），随 WP-MVP-B02 接入"
                ));
            }
        },
        // `03` §4／§5 的受控命令入口。此前 `manage::iam::commands::assign_role`
        // 全仓除测试外没有任何调用方——`user promote` 只建 staff principal、不分配
        // 角色，于是首个站点所有者能登录 /manage 却看不到任何队列。这条命令补上
        // 那一环，并保持与 `user promote` 同级的约束：只能由持迁移凭据的运维在
        // 本机执行，成功与被拒都写审计。
        Command::Iam { command } => {
            use platform::audit::{
                AuditAction, AuditDetails, AuditEvent, AuditReason, AuditResult, AuditSummary,
                AuditTarget, Classification, TargetKind,
            };
            let pool = platform_db::Pool::connect_existing(&config.database)
                .await
                .map_err(|err| err.to_string())?;
            let renderer =
                platform::identifiers::SchemaRenderer::new(&config.database.schema_prefix)
                    .map_err(|err| err.to_string())?;
            let now = chrono::Utc::now();
            let mut tx = platform_db::transaction::DbTransaction::begin(&pool)
                .await
                .map_err(|err| err.to_string())?;
            let outcome = async {
                let mut c = tx.connection();
                let lookup = |name: String| {
                    let renderer = renderer.clone();
                    async move { (name, renderer) }
                };
                let _ = lookup;
                match command {
                    IamCommand::Grant {
                        username,
                        role,
                        by,
                        scope,
                        forum,
                    } => {
                        // 职责分离：`03` §4 要求授权由他人执行。自授在这里直接
                        // 拒绝，不留「先给自己加权再操作」的路径。
                        if username.eq_ignore_ascii_case(&by) {
                            return Err("授予人不得与被授予人相同（职责分离）".to_owned());
                        }
                        let target =
                            identity::management::account_id_by_username(&mut c, &renderer, &username)
                                .await
                                .map_err(|err| err.to_string())?
                                .ok_or_else(|| "被授予账号不存在".to_owned())?;
                        let actor =
                            identity::management::account_id_by_username(&mut c, &renderer, &by)
                                .await
                                .map_err(|err| err.to_string())?
                                .ok_or_else(|| "授予人账号不存在".to_owned())?;
                        let mut forum_ids = Vec::new();
                        for slug in &forum {
                            let table = renderer
                                .table(c.backend(), "community", "forums")
                                .map_err(|err| err.to_string())?;
                            let rows = c
                                .fetch_all(
                                    &format!("SELECT id FROM {table} WHERE slug = ?"),
                                    &[platform_db::transaction::DbValue::Text(slug.clone())],
                                )
                                .await
                                .map_err(|err| err.to_string())?;
                            let row = rows.first().ok_or_else(|| format!("版块不存在：{slug}"))?;
                            forum_ids.push(row.i64("id").map_err(|err| err.to_string())?);
                        }
                        let input = manage::iam::commands::Assignment {
                            user_id: target,
                            role_key: &role,
                            granted_by: actor,
                            scope_type: &scope,
                            forum_ids: &forum_ids,
                            starts_at: now,
                            expires_at: None,
                        };
                        let assigned =
                            manage::iam::commands::assign_role(&mut c, &renderer, &input, now).await;
                        Ok((
                            AuditAction::RoleAssigned,
                            actor,
                            assigned.as_ref().ok().copied().unwrap_or(0),
                            assigned.map(|id| format!("已授予 {username} 角色 {role}（分配 ID {id}，范围 {scope}）；该账号既有管理会话已失效，需重新登录")),
                        ))
                    }
                    IamCommand::Revoke { assignment_id, by } => {
                        let actor =
                            identity::management::account_id_by_username(&mut c, &renderer, &by)
                                .await
                                .map_err(|err| err.to_string())?
                                .ok_or_else(|| "执行人账号不存在".to_owned())?;
                        let table = renderer
                            .table(c.backend(), "iam", "role_assignments")
                            .map_err(|err| err.to_string())?;
                        let rows = c
                            .fetch_all(
                                &format!("SELECT user_id FROM {table} WHERE id = ?"),
                                &[assignment_id.into()],
                            )
                            .await
                            .map_err(|err| err.to_string())?;
                        let holder = rows
                            .first()
                            .ok_or_else(|| "分配不存在".to_owned())?
                            .i64("user_id")
                            .map_err(|err| err.to_string())?;
                        let revoked = manage::iam::commands::revoke_role(
                            &mut c,
                            &renderer,
                            assignment_id,
                            holder,
                            now,
                        )
                        .await;
                        Ok((
                            AuditAction::RoleRevoked,
                            actor,
                            assignment_id,
                            revoked.map(|()| {
                                format!("已撤销分配 {assignment_id}；该账号既有管理会话已失效")
                            }),
                        ))
                    }
                    IamCommand::List { username } => {
                        let target =
                            identity::management::account_id_by_username(&mut c, &renderer, &username)
                                .await
                                .map_err(|err| err.to_string())?
                                .ok_or_else(|| "账号不存在".to_owned())?;
                        let facts = manage::iam::repo::assignments(&mut c, &renderer, target)
                            .await
                            .map_err(|err| err.to_string())?;
                        // 只读查询不写审计：`03` §8 的敏感读另有专门通道，
                        // 这里读的是授权目录本身，不含个人敏感数据。
                        println!("{username} 的角色分配（共 {} 条）：", facts.len());
                        for f in &facts {
                            println!(
                                "  role={} scope={} forums={:?}",
                                f.role_key, f.scope_type, f.forum_ids
                            );
                        }
                        Err(String::new())
                    }
                }
            }
            .await;
            match outcome {
                // List 用空串表示「已自行输出、无需审计」。
                Err(e) if e.is_empty() => {
                    tx.rollback().await.map_err(|err| err.to_string())?;
                    pool.close().await;
                }
                other => {
                    let (action, actor, target_id, result) = match other {
                        Ok(v) => v,
                        Err(message) => {
                            tx.rollback().await.map_err(|err| err.to_string())?;
                            pool.close().await;
                            return Err(message);
                        }
                    };
                    let succeeded = result.is_ok();
                    let event = AuditEvent {
                        event_id: platform::transaction::new_operation_id(),
                        actor_id: actor,
                        action,
                        target: AuditTarget {
                            kind: TargetKind::RoleAssignment,
                            id: target_id.to_string(),
                        },
                        reason: AuditReason::Correction,
                        ticket_id: None,
                        occurred_at: platform::db::encode_time(now),
                        result: if succeeded {
                            AuditResult::Succeeded
                        } else {
                            AuditResult::Denied
                        },
                        operation_id: platform::transaction::new_operation_id(),
                        trace_id: None,
                        version: 1,
                        details: AuditDetails {
                            approval_ids: Vec::new(),
                            before: None,
                            after: Some(AuditSummary::from_value(
                                Classification::C0,
                                target_id.to_string().as_bytes(),
                            )),
                        },
                        correction_of: None,
                    };
                    platform_db::audit_repo::append_audit(&mut tx.connection(), &renderer, &event)
                        .await
                        .map_err(|err| err.to_string())?;
                    tx.commit().await.map_err(|err| err.to_string())?;
                    pool.close().await;
                    match result {
                        Ok(message) => {
                            eprintln!("[ALERT] IAM 授权变更 已执行");
                            println!("{message}");
                        }
                        Err(err) => return Err(err.to_string()),
                    }
                }
            }
        }
        Command::Flag { command } => match command {
            FlagCommand::List => {
                use platform::flags::FlagReader;
                let pool = platform_db::Pool::connect_existing(&config.database)
                    .await
                    .map_err(|err| err.to_string())?;
                let renderer =
                    platform::identifiers::SchemaRenderer::new(&config.database.schema_prefix)
                        .map_err(|err| err.to_string())?;
                let reader = platform_db::flags_repo::DbFlagReader::new(pool.clone(), renderer);
                let result = reader.load().await;
                pool.close().await;
                let snapshot = result.map_err(|err| err.to_string())?;
                println!("Feature Flag 数据库快照（只读；注册默认值单列）：");
                for value in snapshot.values() {
                    println!(
                        "  {:<20} 默认 {} 当前 {} 版本 {} 来源 {}",
                        value.flag.key(),
                        if value.flag.default_enabled() {
                            "开"
                        } else {
                            "关"
                        },
                        if value.enabled { "开" } else { "关" },
                        value
                            .version
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".into()),
                        if value.version.is_some() {
                            "数据库"
                        } else {
                            "缺项安全关闭"
                        }
                    );
                }
            }
            FlagCommand::Set { name, value } => {
                not_implemented(&format!("flag set {name}={value}，随 WP-MVP-A02 接入"));
            }
        },
    }
    Ok(())
}

fn not_implemented(what: &str) -> ! {
    eprintln!("尚未实现：{what}");
    std::process::exit(1);
}
