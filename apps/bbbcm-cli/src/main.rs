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
    /// 列出全部 flag 及其默认状态
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
            UserCommand::Promote { username } => {
                not_implemented(&format!("user promote（{username}），随 WP-MVP-D04 接入"));
            }
            UserCommand::ResetPassword { username } => {
                not_implemented(&format!(
                    "user reset-password（{username}），随 WP-MVP-B02 接入"
                ));
            }
        },
        Command::Flag { command } => match command {
            FlagCommand::List => {
                // 打印的是编译期注册表的默认值，不是数据库中的生效状态——
                // 持久化与受控修改随 WP-MVP-A02 接入（DEC-028 定义两个受控入口）。
                println!("Feature Flag 注册表（默认值；数据库生效状态随 WP-MVP-A02 接入）：");
                for flag in platform::flags::Flag::ALL {
                    println!(
                        "  {:<20} 默认 {}",
                        flag.key(),
                        if flag.default_enabled() { "开" } else { "关" }
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
