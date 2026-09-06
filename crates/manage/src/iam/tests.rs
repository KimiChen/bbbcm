use super::*;

/// CR-011／DEC-058：SiteOwner 恰好四类极高危运维权限，不含任何日常社区权限。
///
/// 这条测试是把决策钉死在代码里：任何人给 site_owner 加一条内容或用户权限，
/// 都会在这里立刻失败，而不是等到线上出现「所有者顺手删了个帖」才发现。
#[test]
fn site_owner_holds_exactly_the_four_approved_high_risk_permissions() {
    let owner = role("site_owner").expect("registry 必须登记 site_owner");
    let mut held: Vec<_> = owner.permissions.to_vec();
    held.sort_unstable();
    assert_eq!(
        held,
        vec![
            "iam.billing.manage",
            "iam.disaster.recover",
            "iam.keys.rotate",
            "iam.production.access",
        ]
    );
    // 不含任何内容、用户、审核或客服权限。
    for key in owner.permissions {
        assert!(
            key.starts_with("iam."),
            "site_owner 不应持有非 iam 权限：{key}"
        );
    }
    // 四项全部标记为高危，因此使用时都要走双人审批。
    for key in owner.permissions {
        assert!(permission(key).unwrap().high_risk, "{key} 应为高危");
    }
}

/// 闭合 registry：未登记的角色或权限一律查不到，也不因字符串合理而放行。
#[test]
fn registry_is_closed_and_rejects_any_wildcard_or_unknown_key() {
    assert!(permission("moderation.enforcement.account").is_some());
    assert!(permission("moderation.enforcement.everything").is_none());
    assert!(role("root").is_none());
    // 通配一律不成立。构建期已拒绝在 registry 里写通配，这里再确认运行时也不接受。
    for wildcard in ["*", "moderation.*", "iam.*"] {
        assert!(permission(wildcard).is_none());
        assert!(!role_grants("administrator", wildcard));
    }
    // 权限键统一为三段命名，且每个权限都声明了资源类型。
    for p in PERMISSIONS {
        assert_eq!(p.key.split('.').count(), 3, "{}", p.key);
        assert!(!p.resource.is_empty(), "{}", p.key);
    }
    // 角色引用的权限必须都在 registry 中。
    for r in ROLES {
        for key in r.permissions {
            assert!(permission(key).is_some(), "角色 {} 引用 {key}", r.key);
        }
        assert!(
            !r.scopes.is_empty(),
            "角色 {} 必须声明允许的范围类型",
            r.key
        );
    }
}

/// 角色能力与范围的边界。
#[test]
fn role_capabilities_and_scopes_match_the_approved_separation_of_duties() {
    // 版主必须绑版块范围：全站范围的版主应当授予 global_moderator。
    assert!(role_allows_scope("moderator", "forums"));
    assert!(!role_allows_scope("moderator", "global"));
    assert!(role_allows_scope("global_moderator", "global"));

    // 账号级处置属高危，版主没有；全局版主与管理员有。
    assert!(!role_grants("moderator", "moderation.enforcement.account"));
    assert!(role_grants(
        "global_moderator",
        "moderation.enforcement.account"
    ));
    assert!(role_grants(
        "administrator",
        "moderation.enforcement.account"
    ));
    assert!(
        permission("moderation.enforcement.account")
            .unwrap()
            .high_risk
    );

    // 只读审计员没有任何写权限。
    let auditor = role("auditor").unwrap();
    for key in auditor.permissions {
        assert!(key.ends_with(".read"), "只读审计员不应持有写权限：{key}");
    }

    // 管理员不持有 SiteOwner 的四类极高危运维权限——职责分离的核心。
    for key in [
        "iam.keys.rotate",
        "iam.billing.manage",
        "iam.production.access",
        "iam.disaster.recover",
    ] {
        assert!(!role_grants("administrator", key), "管理员不应持有 {key}");
    }
}

/// 种子对稳定且与代码常量同源：迁移与启动校验都用它。
#[test]
fn seed_pairs_are_stable_and_cover_every_role_permission() {
    let pairs = seed_pairs();
    assert_eq!(pairs, seed_pairs(), "顺序必须稳定，否则种子会反复漂移");
    let expected: usize = ROLES.iter().map(|r| r.permissions.len()).sum();
    assert_eq!(pairs.len(), expected);
    assert!(pairs.windows(2).all(|w| w[0] <= w[1]), "必须按稳定顺序排列");
}
