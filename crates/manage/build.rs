//! 构建期从 `iam/registry.toml` 生成角色与权限的 Rust 常量（DEC-058／CR-013）。
//!
//! 生成而不是手写，是为了让「代码里的枚举」与「registry 文件」在编译期就不可能分叉；
//! 数据库种子同样由这份文件导出，运行时再校验三者一致。
use std::{collections::BTreeMap, env, fs, path::Path};

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let source = Path::new(&manifest).join("iam/registry.toml");
    println!("cargo:rerun-if-changed={}", source.display());
    let text = fs::read_to_string(&source).expect("读取 registry.toml");
    let parsed: Registry = toml::from_str(&text).expect("解析 registry.toml");

    // 权限键必须唯一且按 `领域.资源.动作` 三段命名。
    let mut permissions = BTreeMap::new();
    for permission in &parsed.permission {
        assert_eq!(
            permission.key.split('.').count(),
            3,
            "权限键必须为三段：{}",
            permission.key
        );
        assert!(
            permissions
                .insert(permission.key.clone(), permission.clone())
                .is_none(),
            "权限键重复：{}",
            permission.key
        );
    }
    // 角色引用的权限必须存在；禁止任何形式的通配（CR-011）。
    for role in &parsed.role {
        for key in &role.permissions {
            assert!(
                key != "*" && !key.ends_with(".*"),
                "禁止权限通配：角色 {} 引用 {}",
                role.key,
                key
            );
            assert!(
                permissions.contains_key(key),
                "角色 {} 引用了未登记的权限 {}",
                role.key,
                key
            );
        }
        for scope in &role.scopes {
            assert!(
                ["global", "forums"].contains(&scope.as_str()),
                "角色 {} 使用了本轮未实现的范围类型 {scope}",
                role.key
            );
        }
    }

    let mut out = String::from(
        "// 本文件由 build.rs 从 iam/registry.toml 生成，请勿手工编辑。\n\
         // 修改角色或权限请改 registry.toml，代码与数据库种子会同步重新生成。\n",
    );
    out.push_str(&format!(
        "pub const PERMISSIONS: &[Permission] = &[\n{}];\n",
        permissions
            .values()
            .map(|p| format!(
                "    Permission {{ key: {:?}, resource: {:?}, high_risk: {} }},\n",
                p.key, p.resource, p.high_risk
            ))
            .collect::<String>()
    ));
    out.push_str(&format!(
        "pub const ROLES: &[Role] = &[\n{}];\n",
        parsed
            .role
            .iter()
            .map(|r| format!(
                "    Role {{ key: {:?}, name: {:?}, scopes: &{:?}, permissions: &{:?} }},\n",
                r.key, r.name, r.scopes, r.permissions
            ))
            .collect::<String>()
    ));
    let target = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("registry.rs");
    fs::write(target, out).expect("写入生成文件");
}

#[derive(serde::Deserialize)]
struct Registry {
    permission: Vec<PermissionEntry>,
    role: Vec<RoleEntry>,
}
#[derive(Clone, serde::Deserialize)]
struct PermissionEntry {
    key: String,
    resource: String,
    high_risk: bool,
    #[allow(dead_code)]
    note: String,
}
#[derive(serde::Deserialize)]
struct RoleEntry {
    key: String,
    name: String,
    scopes: Vec<String>,
    permissions: Vec<String>,
}
