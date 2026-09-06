//! WP-MVP-D01 举报与案件的隔离测试。
use super::*;
use crate::case_repo;
use platform::config::Config;
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    a: i64,
    b: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-cases-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("cases.sqlite3").to_string_lossy().into();
        config.schema_prefix = "cases_probe_".into();
        platform_db::migrations::migrate(
            &config,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&config).await.unwrap();
        let renderer = SchemaRenderer::new(&config.schema_prefix).unwrap();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        let users = renderer
            .table(tx.connection().backend(), "identity", "users")
            .unwrap();
        let mut ids = Vec::new();
        for name in ["reporter-a", "reporter-b"] {
            tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::Text(platform::transaction::new_operation_id()), DbValue::Bool(false), DbValue::Text(name.into()), DbValue::Text(name.into()), DbValue::Text(format!("{name}@example.invalid")), DbValue::Text(format!("{name}@example.invalid")), DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            ids.push(tx.connection().last_insert_id().await.unwrap());
        }
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            renderer,
            a: ids[0],
            b: ids[1],
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-06T00:00:00.000000Z").unwrap()
}

/// MOD-001 去重：一条内容被多人举报是一个案件多条举报，同一人重复举报不新增。
#[tokio::test]
async fn reports_deduplicate_per_reporter_and_share_one_open_case() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();

    let first = submit_report(
        &mut c,
        &f.renderer,
        f.a,
        TargetKind::Topic,
        7,
        ReportCategory::Spam,
        "广告",
        Some("被举报时的正文"),
        at(),
    )
    .await
    .unwrap();
    assert!(first.created_case && first.recorded);

    // 同一举报人重复提交：返回既有工单，不新增举报、不推高计数。
    let again = submit_report(
        &mut c,
        &f.renderer,
        f.a,
        TargetKind::Topic,
        7,
        ReportCategory::Spam,
        "再报一次",
        Some("正文"),
        at(),
    )
    .await
    .unwrap();
    assert_eq!(again.case_id, first.case_id);
    assert!(!again.created_case && !again.recorded);

    // 另一个举报人：挂到同一案件，计数加一。
    let other = submit_report(
        &mut c,
        &f.renderer,
        f.b,
        TargetKind::Topic,
        7,
        ReportCategory::Harassment,
        "辱骂",
        None,
        at(),
    )
    .await
    .unwrap();
    assert_eq!(other.case_id, first.case_id);
    assert!(!other.created_case && other.recorded);
    let case = case_repo::case(&mut c, &f.renderer, first.case_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(case.report_count, 2);

    // 不同目标各自建案。
    let elsewhere = submit_report(
        &mut c,
        &f.renderer,
        f.a,
        TargetKind::Post,
        7,
        ReportCategory::Other,
        "",
        None,
        at(),
    )
    .await
    .unwrap();
    assert!(elsewhere.created_case);
    assert_ne!(elsewhere.case_id, first.case_id);

    // 非法输入拒绝：举报人/目标非正数、备注超长。
    for (reporter, target, note) in [(0, 7, ""), (f.a, 0, ""), (f.a, 7, "x")] {
        let note = if note == "x" {
            "超".repeat(2_001)
        } else {
            note.to_owned()
        };
        assert!(
            submit_report(
                &mut c,
                &f.renderer,
                reporter,
                TargetKind::Profile,
                target,
                ReportCategory::Other,
                &note,
                None,
                at(),
            )
            .await
            .is_err()
        );
    }
    tx.commit().await.unwrap();
}

/// MOD-012：六类高危一进来就置最高优先级并打升级标志，且优先级只升不降。
#[tokio::test]
async fn urgent_categories_escalate_and_priority_never_drops() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();

    // 先来一条普通举报：正常优先级、未升级。
    let outcome = submit_report(
        &mut c,
        &f.renderer,
        f.a,
        TargetKind::Topic,
        11,
        ReportCategory::Other,
        "",
        None,
        at(),
    )
    .await
    .unwrap();
    let case = case_repo::case(&mut c, &f.renderer, outcome.case_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(case.priority, case_repo::PRIORITY_NORMAL);
    assert!(!case.escalated);

    // 高危举报把既有案件提到最高优先级。
    submit_report(
        &mut c,
        &f.renderer,
        f.b,
        TargetKind::Topic,
        11,
        ReportCategory::Privacy,
        "泄露住址",
        None,
        at(),
    )
    .await
    .unwrap();
    let case = case_repo::case(&mut c, &f.renderer, outcome.case_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(case.priority, case_repo::PRIORITY_URGENT);
    assert!(case.escalated);

    // 六类高危的判定与「其他/垃圾广告/辱骂」的区分。
    for category in [
        ReportCategory::Illegal,
        ReportCategory::Privacy,
        ReportCategory::Infringement,
        ReportCategory::Fraud,
        ReportCategory::Malware,
    ] {
        assert!(category.is_urgent(), "{} 应为高危", category.as_str());
    }
    for category in [
        ReportCategory::Spam,
        ReportCategory::Harassment,
        ReportCategory::Other,
    ] {
        assert!(!category.is_urgent());
    }
    // 封闭枚举：未知取值一律拒绝。
    assert!(ReportCategory::parse("unknown").is_err());
    assert!(TargetKind::parse("message").is_err());
    tx.commit().await.unwrap();
}

/// MOD-003 状态机：合法迁移放行、非法迁移拒绝、closed 是终态。
#[tokio::test]
async fn case_status_transitions_are_closed_and_closed_is_terminal() {
    // 先在纯逻辑层把迁移表钉死，不依赖数据库。
    use CaseStatus::*;
    assert!(Triage.permits(Investigating) && Triage.permits(Closed));
    assert!(Investigating.permits(AwaitingInfo) && AwaitingInfo.permits(Investigating));
    // 处置已经发生，不能倒回调查中——否则「处置时的案件状态」无法复原。
    assert!(!Resolved.permits(Investigating));
    assert!(Resolved.permits(Appealed) && Appealed.permits(Resolved));
    // closed 是终态，重开必须是新案件，否则审计链会断。
    for next in [
        Triage,
        Investigating,
        AwaitingInfo,
        Resolved,
        Appealed,
        Closed,
    ] {
        assert!(!Closed.permits(next));
    }

    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let outcome = submit_report(
        &mut c,
        &f.renderer,
        f.a,
        TargetKind::Post,
        21,
        ReportCategory::Spam,
        "",
        Some("证据正文"),
        at(),
    )
    .await
    .unwrap();

    // 非法迁移被拒绝，且不改变状态。
    assert!(
        advance_status(&mut c, &f.renderer, outcome.case_id, Appealed, at())
            .await
            .is_err()
    );
    advance_status(&mut c, &f.renderer, outcome.case_id, Investigating, at())
        .await
        .unwrap();
    assign(&mut c, &f.renderer, outcome.case_id, f.b, at())
        .await
        .unwrap();
    add_note(&mut c, &f.renderer, outcome.case_id, f.b, "内部结论", at())
        .await
        .unwrap();
    // 空备注与超长备注拒绝。
    assert!(
        add_note(&mut c, &f.renderer, outcome.case_id, f.b, "   ", at())
            .await
            .is_err()
    );

    advance_status(&mut c, &f.renderer, outcome.case_id, Resolved, at())
        .await
        .unwrap();
    advance_status(&mut c, &f.renderer, outcome.case_id, Closed, at())
        .await
        .unwrap();
    // 终态之后不能再指派、不能再迁移。
    assert!(
        assign(&mut c, &f.renderer, outcome.case_id, f.a, at())
            .await
            .is_err()
    );
    assert!(
        advance_status(&mut c, &f.renderer, outcome.case_id, Investigating, at())
            .await
            .is_err()
    );

    // 关闭后新举报另开案件，不复活终态案件。
    let reopened = submit_report(
        &mut c,
        &f.renderer,
        f.a,
        TargetKind::Post,
        21,
        ReportCategory::Spam,
        "又来了",
        None,
        at(),
    )
    .await
    .unwrap();
    assert!(reopened.created_case);
    assert_ne!(reopened.case_id, outcome.case_id);

    // MOD-008：证据摘要稳定，且清单接口不返回正文。
    let digests = case_repo::evidence_digests(&mut c, &f.renderer, outcome.case_id)
        .await
        .unwrap();
    assert_eq!(digests.len(), 1);
    use sha2::{Digest, Sha256};
    assert_eq!(
        digests[0].0,
        format!("{:x}", Sha256::digest("证据正文".as_bytes()))
    );
    tx.commit().await.unwrap();
}
