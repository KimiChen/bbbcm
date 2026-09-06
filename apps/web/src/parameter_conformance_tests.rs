//! `WP-MVP-E01`：`90` 第 6 节固定指标表中**参数类门槛**的一致性核验。
//!
//! `90` §6 的表格分两类。负载与延迟门槛（200 read/s、p95 ≤400ms、100 万主题基线…）
//! 需要经批准的资源规格，表末已写明「规格未批准时只能形成实验结果，不能支持
//! Production Approved」，因此属 `WP-MVP-E02`，本文件不涉及。
//!
//! 另一类是**已批准的固定取值**——令牌有效期、配额、长度上限、窗口、反滥用预算。
//! 它们此刻就能权威核验，而且必须核验：这些数字散落在四个 crate 的常量里，
//! 文档与代码各改各的，谁也不会报错。本文件把 `90` §6 的每一个参数钉到实现上，
//! 任何一侧先动都会让这里立刻变红。
//!
//! **这不是行为测试**：它不验证参数被正确执行（那由各领域的用例负责），
//! 只验证「实现里的取值等于计划批准的取值」。两者都需要，缺一不可。
use crate::content_risk::ContentRatePolicy;
use content::contract::Action;
use content::policy::approved_rules;

/// `90` §6：内容长度、标签上限、本人编辑/删除窗口。
///
/// 按 `DEC-055`，长度以 Unicode 标量计，1 标量 = 1 字符——因此这里断言的是标量数，
/// 不是字节数。中日韩字符不做宽度折算。
#[test]
fn content_length_and_window_thresholds_match_the_approved_table() {
    let r = approved_rules();
    assert_eq!(
        (r.minimum_title_chars, r.maximum_title_chars),
        (5, 120),
        "标题 5–120"
    );
    assert_eq!(
        (r.minimum_body_chars, r.maximum_body_chars),
        (10, 50_000),
        "正文 10–50000"
    );
    assert_eq!(
        (r.minimum_reply_chars, r.maximum_reply_chars),
        (2, 10_000),
        "回复 2–10000"
    );
    assert_eq!(r.maximum_tags, 5, "标签 ≤5");
    assert_eq!(r.edit_window_seconds, 30 * 60, "本人编辑窗口 30 分钟");
    assert_eq!(r.delete_window_seconds, 30 * 60, "本人删除窗口 30 分钟");
}

/// `90` §6：T0／T1 发布额度、24 小时新用户窗口、日界线与计额语义。
///
/// 日界线固定为 UTC 零点（`day_utc_offset_minutes == 0`）而不是用户时区：
/// 随时区分日会让同一个人在跨时区旅行时拿到两份额度。
/// `count_pending` 为真对应「发布成功与进入待审均计入」——若不计待审，
/// 只要让内容卡在待审就能无限发。
#[test]
fn publishing_quota_thresholds_match_the_approved_table() {
    let r = approved_rules();
    assert_eq!(
        (r.t0_daily_topics, r.t0_daily_replies),
        (5, 20),
        "T0 每日 5 主题 / 20 回复"
    );
    assert_eq!(
        (r.early_topics, r.early_replies),
        (1, 5),
        "注册 24 小时内 1 主题 / 5 回复"
    );
    assert_eq!(
        (r.t1_daily_topics, r.t1_daily_replies),
        (20, 100),
        "T1 每日 20 主题 / 100 回复"
    );
    assert_eq!(r.day_utc_offset_minutes, 0, "额度日界线按 UTC 零点分日");
    assert!(r.count_pending, "进入待审也必须计入额度");
    // 未验证邮箱／未授级一律不得发布：邮箱验证是所有等级的前置。
    assert!(!r.allow_pending, "pending 等级不得发布");
}

/// `90` §6 + `DEC-060`：外部链接门槛。站内链接不计入，这里只管上限。
#[test]
fn external_link_thresholds_match_the_approved_table() {
    let r = approved_rules();
    assert_eq!(r.max_external_links_t0, 1, "T0 每帖 1 个外链");
    assert_eq!(r.max_external_links_t1, 10, "T1 每帖 10 个外链");
}

/// `90` §6 + `DEC-054`：T1 自动授予门槛。
#[test]
fn trust_tier_promotion_thresholds_match_the_approved_table() {
    assert_eq!(
        trust::core::T1_ACCOUNT_DAYS,
        2,
        "账号存在满 2 个自然日（UTC）"
    );
    assert_eq!(trust::core::T1_CONTRIBUTIONS, 5, "累计 5 条有效贡献");
}

/// `90` §6 + `03` §6：管理会话与高危重新认证。
///
/// 这三个数字直接决定被盗管理会话的可用时长，属于「宁可严不可松」的一类。
#[test]
fn management_session_thresholds_match_the_approved_table() {
    assert_eq!(
        identity::management::MANAGEMENT_ABSOLUTE_HOURS,
        8,
        "管理会话最长 8 小时"
    );
    assert_eq!(
        identity::management::MANAGEMENT_IDLE_MINUTES,
        15,
        "空闲 15 分钟"
    );
    assert_eq!(
        identity::management::ASSERTION_TTL_SECONDS,
        300,
        "高危重新认证 5 分钟内"
    );
}

/// `90` §6 + `DEC-060`：内容四维反滥用预算与网段聚合前缀。
///
/// 四维依次为 账号／IP／设备／网段。编辑与删除各取回复的一半——它们的滥用价值
/// 低于新发内容，但仍需限制，否则可以靠反复编辑刷新排序或规避审核快照。
///
/// 令牌桶语义（`DEC-051`）：容量即所列次数，每 `周期 / 容量` 恢复一个 token，
/// 周期为 1 小时。因此这里连补充间隔一并断言——只对容量不对间隔，
/// 把周期从 1 小时改成 1 天也不会被发现。
#[test]
fn four_dimension_abuse_budgets_match_the_approved_table() {
    const HOUR_MILLIS: u64 = 3_600 * 1_000;
    let expected = |capacity: u32| {
        platform::cache::RateLimitPolicy::new(
            capacity,
            std::time::Duration::from_millis(HOUR_MILLIS / u64::from(capacity)),
        )
        .expect("已批准取值必须构造成功")
    };
    let policy = ContentRatePolicy::approved().expect("已批准预算必须可构造");
    let cases = [
        (Action::CreateTopic, [10_u32, 30, 15, 100], "发主题"),
        (Action::CreateReply, [60, 150, 90, 500], "发回复"),
        (Action::EditPost, [30, 75, 45, 250], "编辑"),
        (Action::DeletePost, [30, 75, 45, 250], "删除"),
    ];
    for (action, [account, ip, device, network], label) in cases {
        let (_, rule) = policy
            .actions
            .iter()
            .find(|(candidate, _)| *candidate == action)
            .unwrap_or_else(|| panic!("{label} 缺少已批准预算"));
        assert_eq!(rule.account, expected(account), "{label} 账号维度");
        assert_eq!(rule.ip, expected(ip), "{label} IP 维度");
        assert_eq!(rule.device, expected(device), "{label} 设备维度");
        assert_eq!(rule.network, expected(network), "{label} 网段维度");
        assert_eq!(rule.ipv4_prefix, 24, "{label} IPv4 网段按 /24 聚合");
        assert_eq!(rule.ipv6_prefix, 64, "{label} IPv6 网段按 /64 聚合");
    }
    assert_eq!(policy.actions.len(), cases.len(), "不得存在表外的动作预算");
}
