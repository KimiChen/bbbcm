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
    /// 写入初始数据（版块、规则、首个管理员）
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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Db { command } => match command {
            DbCommand::Migrate { backend } => {
                if let Some(arg) = backend {
                    let backend: Backend = arg.into();
                    if !backend.is_implemented() {
                        eprintln!(
                            "后端 {backend} 是规范预留值，尚未接入（DEC-022）：\
                             迁移集合 db/migrations/mysql8/ 目前为空占位。"
                        );
                        std::process::exit(1);
                    }
                    not_implemented(&format!(
                        "db migrate --backend {backend}，随 WP-MVP-A03 接入"
                    ));
                }
                not_implemented("db migrate（backend 取自配置），随 WP-MVP-A03 接入");
            }
            DbCommand::Status => not_implemented("db status，随 WP-MVP-A03 接入"),
            DbCommand::Seed => not_implemented("db seed，随 WP-MVP-A01/A03 接入"),
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
}

fn not_implemented(what: &str) -> ! {
    eprintln!("尚未实现：{what}");
    std::process::exit(1);
}
