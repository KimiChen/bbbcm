use super::*;

fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-06T00:00:00.000000Z").unwrap()
}

/// 一条完全有效的全局管理员分配。各测试在此基础上只改一处，用来定位每一步的作用。
fn admin_assignment() -> AssignmentFacts {
    AssignmentFacts {
        role_key: "administrator".into(),
        scope_type: "global".into(),
        forum_ids: Vec::new(),
        starts_at: at() - chrono::Duration::days(1),
        expires_at: None,
        revoked: false,
    }
}
fn healthy() -> AuthorizationFacts {
    AuthorizationFacts {
        account_active: true,
        principal_active: true,
        session_valid: true,
        session_authz_version: 7,
        current_authz_version: 7,
        assignments: vec![admin_assignment()],
        mfa_satisfied: true,
        approval_satisfied: false,
    }
}
fn move_request() -> AuthorizationRequest<'static> {
    AuthorizationRequest {
        actor_id: 1,
        permission_key: "community.content.move",
        resource_type: "content",
        resource_id: 42,
        forum_id: Some(9),
    }
}

/// 基线：完全有效时放行；十步中任何一步不满足都拒绝。
#[test]
fn every_one_of_the_ten_checks_can_deny_on_its_own() {
    assert_eq!(
        authorize(&move_request(), &healthy(), at()),
        Decision::Allowed
    );

    // 第 1–3 步：账号、staff principal、管理会话。
    for mutate in [
        (|f: &mut AuthorizationFacts| f.account_active = false) as fn(&mut AuthorizationFacts),
        |f: &mut AuthorizationFacts| f.principal_active = false,
        |f: &mut AuthorizationFacts| f.session_valid = false,
    ] {
        let mut facts = healthy();
        mutate(&mut facts);
        assert_eq!(authorize(&move_request(), &facts, at()), Decision::Denied);
    }

    // 第 5 步：authz 版本不一致（权限变更后旧会话必须立即失效）。
    let mut stale = healthy();
    stale.session_authz_version = 6;
    assert_eq!(authorize(&move_request(), &stale, at()), Decision::Denied);

    // 第 4 步：未生效、已到期、已撤销的分配都不算数。
    for mutate in [
        (|a: &mut AssignmentFacts| a.starts_at = at() + chrono::Duration::days(1))
            as fn(&mut AssignmentFacts),
        |a: &mut AssignmentFacts| a.expires_at = Some(at() - chrono::Duration::seconds(1)),
        |a: &mut AssignmentFacts| a.revoked = true,
    ] {
        let mut facts = healthy();
        mutate(&mut facts.assignments[0]);
        assert_eq!(authorize(&move_request(), &facts, at()), Decision::Denied);
    }
    // 到期时间恰好等于当前时刻即视为已过期，不给一秒宽限。
    let mut boundary = healthy();
    boundary.assignments[0].expires_at = Some(at());
    assert_eq!(
        authorize(&move_request(), &boundary, at()),
        Decision::Denied
    );

    // 第 6 步：角色不持有该权限。
    let mut auditor = healthy();
    auditor.assignments[0].role_key = "auditor".into();
    assert_eq!(authorize(&move_request(), &auditor, at()), Decision::Denied);

    // 第 6 步续：未登记的权限键与未登记的角色一律拒绝。
    let mut unknown = move_request();
    unknown.permission_key = "moderation.enforcement.everything";
    assert_eq!(authorize(&unknown, &healthy(), at()), Decision::Denied);
    let mut ghost = healthy();
    ghost.assignments[0].role_key = "root".into();
    assert_eq!(authorize(&move_request(), &ghost, at()), Decision::Denied);

    // 第 7 步：权限与资源类型不匹配。
    let mut mismatched = move_request();
    mismatched.resource_type = "account";
    assert_eq!(authorize(&mismatched, &healthy(), at()), Decision::Denied);

    // 第 9 步：MFA 未满足。
    let mut no_mfa = healthy();
    no_mfa.mfa_satisfied = false;
    assert_eq!(authorize(&move_request(), &no_mfa, at()), Decision::Denied);
}

/// ABAC：版块范围按目标内容当前所属版块验证，移帖后覆盖关系随之改变（03 §5）。
#[test]
fn forum_scope_follows_the_targets_current_forum() {
    let mut facts = healthy();
    facts.assignments[0] = AssignmentFacts {
        role_key: "moderator".into(),
        scope_type: "forums".into(),
        forum_ids: vec![9, 11],
        ..admin_assignment()
    };
    // 目标在授权版块内：放行。
    assert_eq!(authorize(&move_request(), &facts, at()), Decision::Allowed);
    // 目标已被移到范围外的版块：同一条分配立即不再覆盖。
    let mut elsewhere = move_request();
    elsewhere.forum_id = Some(12);
    assert_eq!(authorize(&elsewhere, &facts, at()), Decision::Denied);
    // 目标没有版块归属（账号级资源）时，版块范围不覆盖。
    let mut no_forum = move_request();
    no_forum.forum_id = None;
    assert_eq!(authorize(&no_forum, &facts, at()), Decision::Denied);

    // registry 声明版主只能绑版块范围：给版主一个 global 分配不成立。
    let mut illegal = facts.clone();
    illegal.assignments[0].scope_type = "global".into();
    illegal.assignments[0].forum_ids.clear();
    assert_eq!(authorize(&move_request(), &illegal, at()), Decision::Denied);

    // 未实现的范围类型一律不覆盖，而不是退化成 global。
    let mut unknown_scope = facts;
    unknown_scope.assignments[0].scope_type = "case".into();
    assert_eq!(
        authorize(&move_request(), &unknown_scope, at()),
        Decision::Denied
    );
}

/// 03 §7：高危权限必须有一条已通过、且审批人不是操作者本人的审批。
#[test]
fn high_risk_permissions_require_a_separate_approval() {
    let request = AuthorizationRequest {
        actor_id: 1,
        permission_key: "moderation.enforcement.account",
        resource_type: "account",
        resource_id: 5,
        forum_id: None,
    };
    // 其余条件全部满足，但缺审批：拒绝。
    assert_eq!(authorize(&request, &healthy(), at()), Decision::Denied);
    let mut approved = healthy();
    approved.approval_satisfied = true;
    assert_eq!(authorize(&request, &approved, at()), Decision::Allowed);

    // 非高危权限不因缺审批被拒——审批只加在真正的高危动作上。
    assert!(
        !crate::iam::permission("community.content.move")
            .unwrap()
            .high_risk
    );
    assert_eq!(
        authorize(&move_request(), &healthy(), at()),
        Decision::Allowed
    );

    // 撤销批准后旧操作不得继续执行：approval_satisfied 由调用方按当前状态重读，
    // 这里验证一旦它变回假，同一请求立刻被拒。
    let mut revoked = approved;
    revoked.approval_satisfied = false;
    assert_eq!(authorize(&request, &revoked, at()), Decision::Denied);
}

/// 多条分配时，只要有一条覆盖即放行；全部不覆盖才拒绝。
#[test]
fn multiple_assignments_are_evaluated_independently() {
    let mut facts = healthy();
    facts.assignments = vec![
        AssignmentFacts {
            role_key: "moderator".into(),
            scope_type: "forums".into(),
            forum_ids: vec![3],
            ..admin_assignment()
        },
        AssignmentFacts {
            role_key: "moderator".into(),
            scope_type: "forums".into(),
            forum_ids: vec![9],
            ..admin_assignment()
        },
    ];
    assert_eq!(authorize(&move_request(), &facts, at()), Decision::Allowed);
    facts.assignments[1].forum_ids = vec![10];
    assert_eq!(authorize(&move_request(), &facts, at()), Decision::Denied);
    // 一条空分配列表当然拒绝。
    facts.assignments.clear();
    assert_eq!(authorize(&move_request(), &facts, at()), Decision::Denied);
}
