//! 迁移集合发现与 N 轨编号对齐校验；执行器随后续工作包接入。
//!
//! 对齐规则（`02` 第 6.3 节，`DEC-022`）：`db/migrations/` 下每条轨道是一个后端，
//! 同一编号代表同一批结构变更。已填充的轨道编号集合必须完全一致；
//! 只有 `.gitkeep` 的空占位轨（当前的 `mysql8`）在真实迁移落地前豁免。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 计划中的全部迁移轨道。新增后端时在此登记。
pub const TRACKS: &[&str] = &["postgres", "sqlite", "mysql8"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error("读取 {0} 失败: {1}")]
    Unreadable(String, String),
    #[error("迁移文件名必须匹配 NNNN_描述.sql，收到 {track}/{name}")]
    BadFileName { track: String, name: String },
    #[error("{track} 轨存在重复编号 {number}")]
    DuplicateNumber { track: String, number: String },
    #[error("迁移编号不对齐：{detail}")]
    Misaligned { detail: String },
}

fn is_valid_name(stem: &str) -> bool {
    // NNNN_描述：四位数字 + 下划线 + 非空描述。
    let mut it = stem.chars();
    let digits: String = it.by_ref().take(4).collect();
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    matches!(it.next(), Some('_')) && it.clone().count() > 0
}

/// 读取单条轨道的编号集合。目录不存在视为空轨。
fn numbers_in(root: &Path, track: &str) -> Result<BTreeSet<String>, MigrationError> {
    let dir = root.join(track);
    if !dir.exists() {
        return Ok(BTreeSet::new());
    }
    let entries = std::fs::read_dir(&dir)
        .map_err(|err| MigrationError::Unreadable(dir.display().to_string(), err.to_string()))?;
    let mut numbers = BTreeSet::new();
    for entry in entries {
        let path = entry
            .map_err(|err| MigrationError::Unreadable(dir.display().to_string(), err.to_string()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue; // .gitkeep、README 等非迁移文件
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
            MigrationError::BadFileName {
                track: track.to_string(),
                name: path.display().to_string(),
            }
        })?;
        if !is_valid_name(stem) {
            return Err(MigrationError::BadFileName {
                track: track.to_string(),
                name: format!("{stem}.sql"),
            });
        }
        let number = stem[..4].to_string();
        if !numbers.insert(number.clone()) {
            return Err(MigrationError::DuplicateNumber {
                track: track.to_string(),
                number,
            });
        }
    }
    Ok(numbers)
}

/// 校验全部已填充轨道的编号一一对应；返回公共编号集合（升序）。
///
/// 空轨（`mysql8` 占位）豁免对齐，但一旦放入第一个 `.sql` 就必须与其他轨完全一致。
pub fn verify_alignment(root: &Path, tracks: &[&str]) -> Result<Vec<String>, MigrationError> {
    let mut filled: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for track in tracks {
        let numbers = numbers_in(root, track)?;
        if !numbers.is_empty() {
            filled.insert(track, numbers);
        }
    }
    let mut it = filled.iter();
    let Some((first_track, first)) = it.next() else {
        return Ok(Vec::new()); // 全空：尚未开始写迁移
    };
    for (track, numbers) in it {
        if numbers != first {
            let missing: Vec<_> = first.difference(numbers).cloned().collect();
            let extra: Vec<_> = numbers.difference(first).cloned().collect();
            return Err(MigrationError::Misaligned {
                detail: format!("{track} 相对 {first_track} 缺 {missing:?}、多 {extra:?}"),
            });
        }
    }
    Ok(first.iter().cloned().collect())
}

/// 对仓库默认的 `db/migrations/` 与全部登记轨道做校验。
pub fn verify_repository(root: &Path) -> Result<Vec<String>, MigrationError> {
    verify_alignment(root, TRACKS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("bbbcm-mig-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for t in TRACKS {
            fs::create_dir_all(root.join(t)).expect("建轨道目录失败");
        }
        root
    }

    fn write(root: &Path, rel: &str) {
        fs::write(root.join(rel), "-- test").expect("写迁移文件失败");
    }

    #[test]
    fn paired_sets_pass_with_empty_track_exempt() {
        let root = temp_root("ok");
        for t in ["postgres", "sqlite"] {
            write(&root, &format!("{t}/0001_init.sql"));
            write(&root, &format!("{t}/0002_users.sql"));
        }
        // mysql8 只有占位，豁免对齐。
        fs::write(root.join("mysql8/.gitkeep"), "").expect("写占位失败");
        assert_eq!(
            verify_repository(&root).expect("应对齐"),
            vec!["0001", "0002"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn misaligned_sets_fail() {
        let root = temp_root("bad");
        write(&root, "postgres/0001_init.sql");
        write(&root, "sqlite/0001_init.sql");
        write(&root, "sqlite/0002_extra.sql");
        assert!(matches!(
            verify_repository(&root),
            Err(MigrationError::Misaligned { .. })
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filled_third_track_must_align() {
        let root = temp_root("three");
        for t in TRACKS {
            write(&root, &format!("{t}/0001_init.sql"));
        }
        assert_eq!(verify_repository(&root).expect("应对齐"), vec!["0001"]);
        write(&root, "mysql8/0002_only_here.sql");
        assert!(
            matches!(
                verify_repository(&root),
                Err(MigrationError::Misaligned { .. })
            ),
            "一旦填充就不再豁免"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_file_name_is_rejected() {
        let root = temp_root("name");
        write(&root, "postgres/init.sql");
        assert!(matches!(
            verify_repository(&root),
            Err(MigrationError::BadFileName { .. })
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_number_in_same_track_is_rejected() {
        let root = temp_root("dup");
        write(&root, "postgres/0001_init.sql");
        write(&root, "postgres/0001_other.sql");
        assert!(matches!(
            verify_repository(&root),
            Err(MigrationError::DuplicateNumber { .. })
        ));
        let _ = fs::remove_dir_all(&root);
    }

    /// 对仓库真实目录跑一次：当前三轨皆空，应平凡通过；
    /// 一旦有人只往一条轨加迁移，这个测试就会失败。
    #[test]
    fn repository_migrations_are_aligned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
        verify_repository(&root).expect("仓库迁移轨道必须对齐");
    }
}
