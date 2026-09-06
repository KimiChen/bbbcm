//! Isolated ordinary accounts, real registration/verification and app authority.
//! All content policy numbers and the trust assignment below are SYNTHETIC fixtures.
use crate::{
    content_authority::ApplicationContentAuthority,
    identity_web::runtime_tests::{Fixture, token},
};
use chrono::{Duration, Utc};
use content::http::RequestContext;
use content::{
    contract::*,
    core::ContentService,
    cursor::CursorKey,
    policy::{ContentRules, SyntheticContentPolicy, UnconfiguredContentPolicy},
};
use platform::{analytics::AnalyticsFact, transaction::new_operation_id};
use platform_db::transaction::{DbTransaction, DbValue};
use std::sync::Arc;

fn risk_factory(version: i64) -> crate::content_risk::ContentRiskFactory {
    use crate::content_risk::{ContentRatePolicy, ContentRiskFactory, RateRule};
    use platform::cache::{DisabledRateLimiter, RateLimitPolicy};
    let budget = RateLimitPolicy::new(20, std::time::Duration::from_secs(1)).unwrap();
    let rule = RateRule {
        account: budget,
        ip: budget,
        device: budget,
        network: budget,
        ipv4_prefix: 24,
        ipv6_prefix: 64,
    };
    ContentRiskFactory::new(
        vec![3; 32],
        false,
        Arc::new(DisabledRateLimiter),
        Some(ContentRatePolicy {
            version,
            actions: [
                Action::CreateTopic,
                Action::CreateReply,
                Action::EditPost,
                Action::DeletePost,
            ]
            .into_iter()
            .map(|a| (a, rule.clone()))
            .collect(),
        }),
        false,
    )
    .unwrap()
}

fn rules() -> ContentRules {
    ContentRules {
        version: 1,
        allow_pending: false,
        allow_t0: true,
        allow_t1: true,
        minimum_title_chars: 1,
        maximum_title_chars: 200,
        minimum_body_chars: 1,
        maximum_body_chars: 20000,
        minimum_reply_chars: 1,
        maximum_reply_chars: 20000,
        maximum_tags: 5,
        edit_window_seconds: 3600,
        delete_window_seconds: 3600,
        first_post_deletion: content::policy::FirstPostDeletion::HideTopic,
        t0_daily_topics: 5,
        t0_daily_replies: 20,
        t1_daily_topics: 50,
        t1_daily_replies: 200,
        early_topics: 1,
        early_replies: 5,
        day_utc_offset_minutes: 0,
        count_pending: true,
        max_external_links_t0: 0,
        max_external_links_t1: 4,
        show_structural_reply_count: true,
        show_last_public_reply: true,
    }
}
fn service(f: &Fixture) -> ContentService {
    ContentService::new(
        f.pool.clone(),
        f.renderer.clone(),
        Arc::new(ApplicationContentAuthority::synthetic(f.renderer.clone())),
        Arc::new(SyntheticContentPolicy(rules())),
        CursorKey::new(vec![7; 32]).unwrap(),
    )
}
fn credentials(grant: &identity::contract::SessionGrant) -> Credentials {
    Credentials::new(
        grant.token.expose(),
        grant.csrf.expose(),
        new_operation_id(),
    )
}
async fn flags(f: &Fixture, enabled: bool) {
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let t = f
        .renderer
        .table(c.backend(), "platform", "feature_flags")
        .unwrap();
    for key in ["topic_creation", "reply_creation"] {
        c.execute(
            &format!("UPDATE {t} SET enabled = ?, version = version + 1 WHERE flag_key = ?"),
            &[DbValue::Bool(enabled), key.into()],
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}
async fn seed_content(f: &Fixture, user_id: i64) -> i64 {
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let t = f
        .renderer
        .table(c.backend(), "trust", "identity_snapshots")
        .unwrap();
    // Test authority ONLY: normal runtime has no automatic grant or privileged entry.
    c.execute(
        &format!("UPDATE {t} SET level = ?, version = version + 1 WHERE user_id = ?"),
        &["t0".into(), user_id.into()],
    )
    .await
    .unwrap();
    let forums = f
        .renderer
        .table(c.backend(), "community", "forums")
        .unwrap();
    c.execute(&format!("INSERT INTO {forums} (slug, name, position, status, version, parent_id, description, icon, visibility) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"),&["synthetic-content".into(),"合成内容版块".into(),999.into(),"active".into(),1.into(),0.into(),"仅隔离测试".into(),"".into(),"public".into()]).await.unwrap();
    let id = c.last_insert_id().await.unwrap();
    let rules = f
        .renderer
        .table(c.backend(), "moderation", "rule_versions")
        .unwrap();
    c.execute(
        &format!("INSERT INTO {rules} (forum_id, version, body, status) VALUES (?, ?, ?, ?)"),
        &[
            id.into(),
            1.into(),
            "仅隔离夹具的合成版规".into(),
            "published".into(),
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}
async fn scalar(f: &Fixture, domain: &str, table: &str, field: &str) -> i64 {
    assert!(matches!(field, "COUNT(*)"));
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let t = f
        .renderer
        .table(tx.connection().backend(), domain, table)
        .unwrap();
    let value = tx
        .connection()
        .fetch_optional(&format!("SELECT {field} AS n FROM {t}"), &[])
        .await
        .unwrap()
        .unwrap()
        .i64("n")
        .unwrap();
    tx.rollback().await.unwrap();
    value
}

#[tokio::test]
async fn ordinary_account_real_content_transaction_projection_replay_and_worker_recovery() {
    let f = Fixture::new().await;
    f.enable().await;
    flags(&f, true).await;
    f.service
        .register(&f.context().await, f.registration().await, Utc::now())
        .await
        .unwrap();
    let (grant, identity) = f.login("Synthetic orchard password 734!").await;
    let content = service(&f);
    let draft = NewTopic {
        forum_id: 1,
        title: "合成主题".into(),
        markdown: "安全 **正文**".into(),
        tag_ids: vec![],
        visibility: Visibility::Public,
    };
    assert!(matches!(
        content
            .create_topic(&credentials(&grant), draft.clone(), Utc::now())
            .await,
        Err(ContentError::Forbidden)
    ));
    f.service
        .consume_verification(
            &f.context().await,
            &token(&f.message("verify_email").await.1),
            Utc::now(),
        )
        .await
        .unwrap();
    // DEC-054：验证通过即授予 T0，信任等级本身不再拦截发布。
    // 但此刻仍应失败——原因不是授权，而是版块尚未建立，因此期望 NotFound 而非 Forbidden。
    // 这样区分可以证明「等级放行了」而不是「换了个地方被拒」。
    let after_verification = content
        .create_topic(&credentials(&grant), draft.clone(), Utc::now())
        .await;
    assert!(
        matches!(after_verification, Err(ContentError::NotFound)),
        "unexpected outcome after verification: {after_verification:?}"
    );
    let forum_id = seed_content(&f, identity.user_id()).await;
    let draft = NewTopic { forum_id, ..draft };
    let mut create = credentials(&grant);
    create.trusted_context = Some(
        risk_factory(1)
            .prepare(
                &axum::http::HeaderMap::new(),
                None,
                identity.user_id(),
                Action::CreateTopic,
            )
            .await
            .unwrap(),
    );
    let result = content
        .create_topic(&create, draft.clone(), Utc::now())
        .await
        .unwrap();
    let replay = content
        .create_topic(&create, draft.clone(), Utc::now())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(result.post_id, replay.post_id);
    assert_eq!(scalar(&f, "community", "topics", "COUNT(*)").await, 1);
    assert_eq!(
        scalar(&f, "community", "search_documents", "COUNT(*)").await,
        1
    );
    let detail = content
        .topic(result.topic_id, None, None, Utc::now())
        .await
        .unwrap();
    assert_eq!(detail.topic.forum.rule, "仅隔离夹具的合成版规");
    assert!(detail.first_post.html.contains("<strong>正文</strong>"));
    assert_eq!(detail.topic.reply_count, Some(0));
    let reply = content
        .reply(
            &credentials(&grant),
            NewReply {
                topic_id: result.topic_id,
                markdown: "合成回复".into(),
                quote_post_id: Some(result.post_id),
                expected_topic_version: detail.topic.version,
            },
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(reply.floor_no, 2);
    let edited = content
        .edit(
            &credentials(&grant),
            EditPost {
                post_id: result.post_id,
                expected_version: 1,
                title: Some("本人修订".into()),
                markdown: "修订后正文".into(),
                tag_ids: Some(vec![]),
                visibility: Some(Visibility::Members),
            },
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(edited.version, 2);
    assert!(matches!(
        content.topic(result.topic_id, None, None, Utc::now()).await,
        Err(ContentError::NotFound)
    ));
    let member = content
        .topic(
            result.topic_id,
            None,
            Some(grant.token.expose()),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(member.first_post.markdown, "修订后正文");
    assert_eq!(member.replies.len(), 1);
    assert!(member.first_post.edited_at.is_some());
    assert_eq!(
        scalar(&f, "community", "search_documents", "COUNT(*)").await,
        2
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let t = f
        .renderer
        .table(tx.connection().backend(), "community", "search_documents")
        .unwrap();
    let rows = tx
        .connection()
        .fetch_all(&format!("SELECT visibility FROM {t}"), &[])
        .await
        .unwrap();
    assert!(
        rows.iter()
            .all(|r| r.text("visibility").unwrap() == "members")
    );
    let t = f
        .renderer
        .table(tx.connection().backend(), "analytics", "event_facts")
        .unwrap();
    let rows = tx
        .connection()
        .fetch_all(
            &format!("SELECT fact_json FROM {t} WHERE event_kind IN (?, ?)"),
            &["topic_published".into(), "reply_published".into()],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let fact = AnalyticsFact::from_storage_json(&row.text("fact_json").unwrap()).unwrap();
        assert!(fact.excluded());
    }
    tx.rollback().await.unwrap();
    let count = scalar(&f, "analytics", "event_facts", "COUNT(*)").await;
    f.drain(&f.config).await;
    let processed = scalar(&f, "analytics", "projection_receipts", "COUNT(*)").await;
    assert_eq!(processed, count);
    f.drain(&f.config).await;
    assert_eq!(
        scalar(&f, "analytics", "projection_receipts", "COUNT(*)").await,
        processed
    );
    assert_eq!(scalar(&f, "analytics", "aggregates", "COUNT(*)").await, 0);
    // Old requests cannot bypass CSRF or a disabled write flag even on replay.
    let mut bad = create.clone();
    bad.csrf = SecretText::new("wrong");
    assert!(
        content
            .create_topic(&bad, draft.clone(), Utc::now())
            .await
            .is_err()
    );
    flags(&f, false).await;
    assert!(
        content
            .create_topic(&create, draft.clone(), Utc::now())
            .await
            .is_err()
    );
    flags(&f, true).await;
    f.service
        .logout(
            &identity,
            grant.csrf.expose(),
            &new_operation_id(),
            Utc::now(),
        )
        .await
        .unwrap();
    assert!(
        content
            .create_topic(&create, draft, Utc::now())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn normal_policy_is_closed_and_current_restrictions_reject_without_partial_content() {
    let f = Fixture::new().await;
    f.enable().await;
    flags(&f, true).await;
    f.service
        .register(&f.context().await, f.registration().await, Utc::now())
        .await
        .unwrap();
    f.service
        .consume_verification(
            &f.context().await,
            &token(&f.message("verify_email").await.1),
            Utc::now(),
        )
        .await
        .unwrap();
    let (grant, identity) = f.login("Synthetic orchard password 734!").await;
    let forum = seed_content(&f, identity.user_id()).await;
    let draft = NewTopic {
        forum_id: forum,
        title: "合成限制测试".into(),
        markdown: "正文".into(),
        tag_ids: vec![],
        visibility: Visibility::Public,
    };
    let mut forged = credentials(&grant);
    forged.trusted_context = Some(Arc::new("client-supplied-allow".to_string()));
    assert!(matches!(
        service(&f)
            .create_topic(&forged, draft.clone(), Utc::now())
            .await,
        Err(ContentError::Forbidden)
    ));
    let mut stale = credentials(&grant);
    stale.trusted_context = Some(
        risk_factory(71)
            .prepare(
                &axum::http::HeaderMap::new(),
                None,
                identity.user_id(),
                Action::CreateTopic,
            )
            .await
            .unwrap(),
    );
    assert!(matches!(
        service(&f)
            .create_topic(&stale, draft.clone(), Utc::now())
            .await,
        Err(ContentError::PolicyPending)
    ));
    let mut wrong_actor = credentials(&grant);
    wrong_actor.trusted_context = Some(
        risk_factory(1)
            .prepare(
                &axum::http::HeaderMap::new(),
                None,
                identity.user_id() + 1,
                Action::CreateTopic,
            )
            .await
            .unwrap(),
    );
    assert!(matches!(
        service(&f)
            .create_topic(&wrong_actor, draft.clone(), Utc::now())
            .await,
        Err(ContentError::Forbidden)
    ));
    let closed = ContentService::new(
        f.pool.clone(),
        f.renderer.clone(),
        Arc::new(ApplicationContentAuthority::new(f.renderer.clone())),
        Arc::new(UnconfiguredContentPolicy),
        CursorKey::new(vec![9; 32]).unwrap(),
    );
    assert!(
        closed
            .list(&ListFilter::default(), None, None, Utc::now())
            .await
            .unwrap()
            .topics
            .is_empty()
    );
    assert!(matches!(
        closed
            .create_topic(&credentials(&grant), draft.clone(), Utc::now())
            .await,
        Err(ContentError::PolicyPending)
    ));
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let t = f
        .renderer
        .table(
            tx.connection().backend(),
            "moderation",
            "content_restrictions",
        )
        .unwrap();
    tx.connection().execute(&format!("INSERT INTO {t} (user_id, capability, starts_at, expires_at, revoked, version) VALUES (?, ?, ?, ?, ?, ?)"),&[identity.user_id().into(),"topic".into(),DbValue::Time(Utc::now()-Duration::seconds(1)),DbValue::Time(Utc::now()+Duration::hours(1)),DbValue::Bool(false),1.into()]).await.unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(
        service(&f)
            .create_topic(&credentials(&grant), draft, Utc::now())
            .await,
        Err(ContentError::Forbidden)
    ));
    assert_eq!(scalar(&f, "community", "topics", "COUNT(*)").await, 0);
    assert_eq!(
        scalar(&f, "community", "search_documents", "COUNT(*)").await,
        0
    );
    assert_eq!(
        scalar(&f, "community", "post_revisions", "COUNT(*)").await,
        0
    );
}

#[tokio::test]
async fn real_content_ssr_form_submission_preview_and_member_filter_need_no_javascript() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = Fixture::new().await;
    f.enable().await;
    flags(&f, true).await;
    f.service
        .register(&f.context().await, f.registration().await, Utc::now())
        .await
        .unwrap();
    f.service
        .consume_verification(
            &f.context().await,
            &token(&f.message("verify_email").await.1),
            Utc::now(),
        )
        .await
        .unwrap();
    let (grant, identity) = f.login("Synthetic orchard password 734!").await;
    let forum = seed_content(&f, identity.user_id()).await;
    let assets = crate::assets::Assets::load_for(
        platform::config::Environment::Test,
        &f.dir.join("missing-assets"),
        false,
    )
    .unwrap();
    let mut state = crate::content_web::closed(&f.config, &assets);
    state.service = Some(Arc::new(service(&f)));
    state.flags = Some(Arc::new(platform_db::flags_repo::DbFlagReader::new(
        f.pool.clone(),
        f.renderer.clone(),
    )));
    state.request_context = Some(Arc::new(risk_factory(1)));
    let app: axum::Router = content::http::router(state);
    let cookies = format!(
        "bcm_session={}; bcm_csrf={}",
        grant.token.expose(),
        grant.csrf.expose()
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/topics/new?theme=dark")
                .header("cookie", &cookies)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 1_048_576)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("method=\"post\""));
    assert!(!html.contains("<script"));
    let form = format!(
        "csrf={}&idempotency_key={}&forum_id={forum}&title=Synthetic+SSR&body=%2A%2Asafe%2A%2A&visibility=members&theme=dark",
        grant.csrf.expose(),
        new_operation_id()
    );
    let request = |body: String| {
        Request::builder()
            .uri("/topics/new")
            .method("POST")
            .header("host", "127.0.0.1:18099")
            .header("origin", "http://127.0.0.1:18099")
            .header("cookie", &cookies)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    };
    let preview = app
        .clone()
        .oneshot(request(format!("{form}&intent=preview")))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(preview.into_body(), 1_048_576)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("<strong>safe</strong>"));
    assert_eq!(scalar(&f, "community", "topics", "COUNT(*)").await, 0);
    let posted = app.clone().oneshot(request(form.clone())).await.unwrap();
    assert_eq!(posted.status(), StatusCode::SEE_OTHER);
    let target = posted.headers()["location"].to_str().unwrap().to_string();
    let located = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&target)
                .header("cookie", &cookies)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // DEC-056：楼层永久链接解析出的是权威 `/t/{id}#post-N`，属于同一资源的规范地址，
    // 因此改用 308 永久重定向（保留请求方法），而不是原先的 303。
    assert_eq!(located.status(), StatusCode::PERMANENT_REDIRECT);
    let target = located.headers()["location"]
        .to_str()
        .unwrap()
        .split('#')
        .next()
        .unwrap()
        .to_string();
    let member = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&target)
                .header("cookie", &cookies)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(member.status(), StatusCode::OK);
    assert_eq!(member.headers()["cache-control"], "private, no-store");
    let html = String::from_utf8(
        to_bytes(member.into_body(), 1_048_576)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("<strong>safe</strong>"));
    assert!(html.contains("Synthetic SSR"));
    let anonymous = app
        .clone()
        .oneshot(Request::builder().uri(target).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::NOT_FOUND);
    for path in ["/", "/new", "/pinned", "/featured"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            to_bytes(response.into_body(), 1_048_576)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!html.contains("Synthetic SSR"));
    }
    let duplicate = app.oneshot(request(form)).await.unwrap();
    assert_eq!(duplicate.status(), StatusCode::SEE_OTHER);
    assert_eq!(scalar(&f, "community", "topics", "COUNT(*)").await, 1);
}
