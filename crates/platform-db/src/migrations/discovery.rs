use super::MigrationError;
use platform::db::Backend;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const TRACKS: &[&str] = &["postgres", "sqlite", "mysql8"];

#[derive(Debug, Clone)]
pub struct Migration {
    pub number: i64,
    pub name: String,
    pub checksum: String,
    pub sql: String,
    pub rebuild: bool,
}

#[derive(Debug)]
pub struct MigrationSet {
    pub backend: Backend,
    pub migrations: Vec<Migration>,
}

fn filenames(root: &Path, track: &str) -> Result<BTreeMap<i64, String>, MigrationError> {
    let dir = root.join(track);
    if !dir.exists() && track == "mysql8" {
        return Ok(BTreeMap::new());
    }
    let mut names = BTreeMap::new();
    for entry in std::fs::read_dir(dir).map_err(|_| MigrationError::Unreadable)? {
        let entry = entry.map_err(|_| MigrationError::Unreadable)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| MigrationError::Unreadable)?;
        if matches!(name.as_str(), ".gitkeep" | "README.md") {
            continue;
        }
        let stem = name.strip_suffix(".sql").unwrap_or("");
        let valid = stem.len() > 5
            && stem.as_bytes()[..4].iter().all(u8::is_ascii_digit)
            && stem.as_bytes()[4] == b'_'
            && stem.as_bytes()[5..]
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
            && entry
                .file_type()
                .map_err(|_| MigrationError::Unreadable)?
                .is_file();
        if !valid {
            return Err(MigrationError::BadFileName {
                track: track.to_owned(),
                name,
            });
        }
        let number: i64 = stem[..4].parse().map_err(|_| MigrationError::Unreadable)?;
        if number == 0 {
            return Err(MigrationError::BadFileName {
                track: track.to_owned(),
                name,
            });
        }
        if names.insert(number, name).is_some() {
            return Err(MigrationError::DuplicateNumber {
                track: track.to_owned(),
                number: format!("{number:04}"),
            });
        }
    }
    Ok(names)
}

/// 仅 mysql8 的未填充轨可豁免；有效后端必须非空且从 0001 连续对齐。
pub fn verify_alignment(root: &Path, tracks: &[&str]) -> Result<Vec<String>, MigrationError> {
    let mut canonical: Option<BTreeSet<i64>> = None;
    for track in tracks {
        let numbers: BTreeSet<_> = filenames(root, track)?.into_keys().collect();
        if numbers.is_empty() && *track == "mysql8" {
            continue;
        }
        if numbers.is_empty() || numbers.iter().copied().ne(1..=numbers.len() as i64) {
            return Err(MigrationError::Misaligned {
                detail: format!("{track} 必须非空且从 0001 连续编号"),
            });
        }
        if let Some(first) = &canonical {
            if first != &numbers {
                return Err(MigrationError::Misaligned {
                    detail: format!("{track} 编号集合不同"),
                });
            }
        } else {
            canonical = Some(numbers);
        }
    }
    canonical
        .map(|numbers| numbers.iter().map(|n| format!("{n:04}")).collect())
        .ok_or_else(|| MigrationError::Misaligned {
            detail: "没有有效迁移轨".to_owned(),
        })
}

pub fn verify_repository(root: &Path) -> Result<Vec<String>, MigrationError> {
    verify_alignment(root, TRACKS)
}

/// 获取字符串/注释之外的词与分号；防止迁移绕过 runner 事务或 PRAGMA。
fn tokens(sql: &str) -> Result<Vec<String>, ()> {
    let chars: Vec<_> = sql.chars().collect();
    let mut result = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '-' && chars.get(at + 1) == Some(&'-') {
            at += 2;
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
        } else if c == '/' && chars.get(at + 1) == Some(&'*') {
            at += 2;
            let mut depth = 1;
            while at < chars.len() && depth > 0 {
                if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                    // PostgreSQL 支持嵌套而 SQLite 不支持；预检不能隐藏实际 COMMIT。
                    return Err(());
                } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            if depth != 0 {
                return Err(());
            }
        } else if c == '\'' {
            if at > 0
                && matches!(chars[at - 1], 'e' | 'E')
                && (at < 2 || !(chars[at - 2].is_ascii_alphanumeric() || chars[at - 2] == '_'))
            {
                return Err(());
            }
            at += 1;
            let mut closed = false;
            while at < chars.len() {
                // PG E/U& 字面量和非 standard_conforming_strings 的反斜杠语义不同。
                // 迁移只允许单引号加双写引号，防止词法差异隐藏 COMMIT。
                if chars[at] == '\\' {
                    return Err(());
                }
                if chars[at] == '\'' {
                    if chars.get(at + 1) == Some(&'\'') {
                        at += 2;
                    } else {
                        at += 1;
                        closed = true;
                        break;
                    }
                } else {
                    at += 1;
                }
            }
            if !closed {
                return Err(());
            }
        } else if c == ';' {
            result.push(";".to_owned());
            at += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = at;
            at += 1;
            while at < chars.len() && (chars[at].is_ascii_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            result.push(
                chars[start..at]
                    .iter()
                    .collect::<String>()
                    .to_ascii_uppercase(),
            );
        } else if matches!(c, '"' | '`' | '$' | '\0') {
            return Err(());
        } else {
            at += 1;
        }
    }
    Ok(result)
}

/// `DEC-065`：声明本轨确实无对应语义的迁移标记，必须位于文件首行。
///
/// 只有带此标记的文件才豁免「至少含一条可执行 SQL」。用**显式标记**而不是
/// 「允许某一轨缺号」，是因为文件不存在时无从检查它为何缺席，而 `DEC-065`
/// 要求这条豁免可被机器挡住、不被拿来跳过本应实现的结构变更。
const TRACK_NOOP_MARKER: &str = "-- BBBCM-TRACK-NOOP:";

fn validate_content(number: i64, sql: &str) -> Result<(), MigrationError> {
    let fail = |reason| MigrationError::InvalidContent { number, reason };
    if !sql.ends_with('\n') || sql.contains('\r') {
        return Err(fail("必须使用 UTF-8、LF 和末尾换行"));
    }
    // 标记必须在首行且写明原因，避免它被塞在文件中间当成普通注释混过去。
    let declared_noop = sql.lines().next().is_some_and(|line| {
        line.starts_with(TRACK_NOOP_MARKER) && line.len() > TRACK_NOOP_MARKER.len()
    });
    let tokens = tokens(sql).map_err(|_| fail("未闭合文本、非法引号或非受信 SQL 字面形式"))?;
    let statements: Vec<_> = tokens
        .split(|t| t == ";")
        .filter(|s| !s.is_empty())
        .collect();
    if statements.is_empty() && !declared_noop {
        return Err(fail("迁移必须包含可执行 SQL"));
    }
    if !statements.is_empty() && declared_noop {
        return Err(fail("声明为本轨无操作的迁移不得再含可执行 SQL"));
    }
    for statement in statements {
        if matches!(
            statement[0].as_str(),
            "BEGIN"
                | "ABORT"
                | "PREPARE"
                | "START"
                | "COMMIT"
                | "END"
                | "ROLLBACK"
                | "SAVEPOINT"
                | "RELEASE"
                | "PRAGMA"
                | "ATTACH"
                | "DETACH"
                | "VACUUM"
                | "SET"
                | "RESET"
        ) {
            return Err(fail("事务、连接设置与 PRAGMA 由 runner 独占控制"));
        }
        if statement[0] == "CREATE"
            && statement
                .iter()
                .skip(1)
                .take(3)
                .any(|t| matches!(t.as_str(), "TRIGGER" | "VIEW" | "PROCEDURE" | "FUNCTION"))
        {
            return Err(fail("禁止触发器、视图、存储过程与数据库函数"));
        }
    }
    Ok(())
}

impl MigrationSet {
    pub fn discover(root: &Path, backend: Backend) -> Result<Self, MigrationError> {
        if !backend.is_implemented() {
            return Err(MigrationError::UnsupportedBackend);
        }
        verify_repository(root)?;
        let track = backend.to_string();
        let mut migrations = Vec::new();
        for (number, name) in filenames(root, &track)? {
            let bytes = std::fs::read(root.join(&track).join(&name))
                .map_err(|_| MigrationError::Unreadable)?;
            let sql = String::from_utf8(bytes).map_err(|_| MigrationError::InvalidContent {
                number,
                reason: "必须为 UTF-8",
            })?;
            validate_content(number, &sql)?;
            migrations.push(Migration {
                number,
                name,
                checksum: format!("{:x}", Sha256::digest(sql.as_bytes())),
                rebuild: sql.lines().next() == Some("-- rebuild"),
                sql,
            });
        }
        Ok(Self {
            backend,
            migrations,
        })
    }
}
