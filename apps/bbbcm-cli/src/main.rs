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
    /// 写入 development/test 四版块与规则占位（不创建账号）
    Seed,
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
    Set { name: String, value: bool },
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
