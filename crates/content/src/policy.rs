//! 内容发布策略的显式注入点。
//!
//! 本模块只负责「取得一份已批准的发布规则」，不做任何数据库访问，也不判定单次请求是否
//! 放行——授权判定在 `ContentRules::authorize` 与 `validate_text`，额度消费在 core 的写事务里。
//!
//! 生产运行使用 `ApprovedContentPolicy`，其取值来自 DEC-054（内容产品参数）、DEC-055
//! （Unicode 标量计量）与 DEC-060（外链门槛）。这些值以版本化常量写在代码里而不是配置项：
//! `plan/` 是唯一事实源，把发布策略做成可运行时改写的配置等于给未批准取值开后门。
//!
//! `UnconfiguredContentPolicy` 保留下来，用于「装配不完整就失败关闭」的路径：任何未接入
//! 批准策略的部署一律返回 `PolicyPending`，而不是退化成某个方便的默认值。
//! `SyntheticContentPolicy` 只允许测试用显式标注的合成取值构造，永远不作为运行时默认。
use crate::contract::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstPostDeletion {
    PolicyPending,
    HideTopic,
}
#[derive(Debug, Clone)]
pub struct ContentRules {
    pub version: i64,
    pub allow_pending: bool,
    pub allow_t0: bool,
    pub allow_t1: bool,
    pub minimum_title_chars: usize,
    pub maximum_title_chars: usize,
    pub minimum_body_chars: usize,
    pub maximum_body_chars: usize,
    pub minimum_reply_chars: usize,
    pub maximum_reply_chars: usize,
    pub maximum_tags: usize,
    pub edit_window_seconds: i64,
    pub delete_window_seconds: i64,
    pub first_post_deletion: FirstPostDeletion,
    pub t0_daily_topics: i64,
    pub t0_daily_replies: i64,
    pub t1_daily_topics: i64,
    pub t1_daily_replies: i64,
    pub early_topics: i64,
    pub early_replies: i64,
    pub day_utc_offset_minutes: i32,
    pub count_pending: bool,
    pub max_external_links_t0: usize,
    pub max_external_links_t1: usize,
    /// Structural published replies; this does not declare valid-contribution semantics.
    pub show_structural_reply_count: bool,
    pub show_last_public_reply: bool,
}
pub trait ContentPolicy: Send + Sync {
    fn rules(&self) -> Result<ContentRules, ContentError>;
}
/// 已批准发布规则的版本号。
///
/// 该版本号会随发布回执一起落库，用于事后复核「这条内容是按哪一版规则发布的」。
/// 只要下面 `approved_rules` 的任何一个取值发生变化，就必须同步递增本常量，
/// 且变更前须先按 `plan/06-decisions-and-risks.md` 第 5 节的流程取得项目所有者批准。
pub const APPROVED_RULES_VERSION: i64 = 1;

/// 返回 DEC-054／DEC-055／DEC-060 已批准的发布规则取值。
///
/// 逐项对应关系（改动任何一项都要同时改 `plan/` 与 `APPROVED_RULES_VERSION`）：
///
/// - **信任等级**：邮箱验证通过即自动授予 T0；T1 由 `trust::core::evaluate_tier` 的确定性
///   规则授予。本处只声明「哪些等级可以发布」，不负责授级。
/// - **长度与标签**：标题 5–120、正文 10–50000、回复 2–10000，标签每主题至多 5 个。
///   长度一律按 Unicode 标量计数（DEC-055：1 标量 = 1 字符，不做中日韩宽度折算），
///   与 `USER-004` 简介 500 标量的口径保持一致。
/// - **外链门槛**：T0 每帖至多 1 个站外链接，T1 至多 10 个（DEC-060）。站内链接由调用方
///   在统计外链数量时排除，不计入此上限。
/// - **本人编辑／删除窗口**：均为 30 分钟；超时后本人只能走举报或管理路径。
///   删除主题首帖时整个主题进入删除态（`FirstPostDeletion::HideTopic`），楼层号与回复
///   记录保留，不重排、不回退既有计数。
/// - **严格配额**：T0 每自然日 5 主题 / 20 回复，T1 每自然日 20 主题 / 100 回复；
///   注册后 24 小时窗口另有 1 主题 / 5 回复的上限，与自然日额度取更严者。
/// - **计额语义**：`count_pending = true` 表示进入待审队列同样计入额度，避免刷待审绕过；
///   本人删除不返还当日额度（消费记录行保留，core 的 `quota` 按记录行统计）。
/// - **日界线**：`day_utc_offset_minutes = 0`，即 UTC 零点分日，与 DEC-048 的统计时区
///   和 `INVITE-006` 的邀请日界线一致。
///
/// 注意：这里的严格配额与 `content_risk` 里的四维令牌桶预算是两套并存机制，互不替代——
/// 前者是产品规则，落在主库事务内；后者是反滥用兜底，落在 Redis 令牌桶上。
pub fn approved_rules() -> ContentRules {
    ContentRules {
        version: APPROVED_RULES_VERSION,
        // 邮箱验证是所有等级的前置条件，因此 pending（未验证或尚未授级）不得发布。
        allow_pending: false,
        allow_t0: true,
        allow_t1: true,
        minimum_title_chars: 5,
        maximum_title_chars: 120,
        minimum_body_chars: 10,
        maximum_body_chars: 50_000,
        minimum_reply_chars: 2,
        maximum_reply_chars: 10_000,
        maximum_tags: 5,
        edit_window_seconds: 30 * 60,
        delete_window_seconds: 30 * 60,
        first_post_deletion: FirstPostDeletion::HideTopic,
        t0_daily_topics: 5,
        t0_daily_replies: 20,
        t1_daily_topics: 20,
        t1_daily_replies: 100,
        early_topics: 1,
        early_replies: 5,
        // DEC-048／DEC-054 把统计与配额的日界线固定为 UTC 零点，不随用户时区变化。
        day_utc_offset_minutes: 0,
        count_pending: true,
        // DEC-060 外链门槛。调用方在统计外链数量时已排除站内链接，这里只管上限。
        max_external_links_t0: 1,
        max_external_links_t1: 10,
        // 仅展示结构性计数（已发布回复条数、最后一条未删回复）。DEC-054 定义的
        // 「有效回复数」与浏览量统计属于后续波次，本轮不在此声明其语义。
        show_structural_reply_count: true,
        show_last_public_reply: true,
    }
}

/// 承载已批准取值的正常运行时策略。
///
/// 构造时就跑一次 `ContentRules::validate`，把「常量之间自相矛盾」这类错误挡在应用装配阶段，
/// 而不是拖到第一次发布请求才暴露。装配失败时调用方应当拒绝启动，而不是回退到某个默认值。
pub struct ApprovedContentPolicy(ContentRules);
impl ApprovedContentPolicy {
    pub fn new() -> Result<Self, ContentError> {
        let rules = approved_rules();
        rules.validate()?;
        Ok(Self(rules))
    }
}
impl ContentPolicy for ApprovedContentPolicy {
    fn rules(&self) -> Result<ContentRules, ContentError> {
        Ok(self.0.clone())
    }
}

/// 未配置策略：任何取得规则的尝试都返回 `PolicyPending`。
///
/// 它是「失败关闭」的具体实现——装配不完整的部署不会退化成宽松默认值，而是整体拒绝发布。
pub struct UnconfiguredContentPolicy;
impl ContentPolicy for UnconfiguredContentPolicy {
    fn rules(&self) -> Result<ContentRules, ContentError> {
        Err(ContentError::PolicyPending)
    }
}
/// 合成策略：只允许在测试中用显式标注的合成取值构造，永远不得作为运行时默认。
///
/// 使用它的测试必须自证取值是合成的，不能把合成取值当成已批准取值来宣称功能完成。
pub struct SyntheticContentPolicy(pub ContentRules);
impl ContentPolicy for SyntheticContentPolicy {
    fn rules(&self) -> Result<ContentRules, ContentError> {
        self.0.validate()?;
        Ok(self.0.clone())
    }
}
impl ContentRules {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.version <= 0
            || self.minimum_title_chars == 0
            || self.minimum_title_chars > self.maximum_title_chars
            || self.minimum_body_chars == 0
            || self.minimum_body_chars > self.maximum_body_chars
            || self.minimum_reply_chars == 0
            || self.minimum_reply_chars > self.maximum_reply_chars
            || self.edit_window_seconds < 0
            || self.delete_window_seconds < 0
            || chrono::Duration::try_seconds(self.edit_window_seconds).is_none()
            || chrono::Duration::try_seconds(self.delete_window_seconds).is_none()
            || self.day_utc_offset_minutes.unsigned_abs() > 840
            || [
                self.t0_daily_topics,
                self.t0_daily_replies,
                self.t1_daily_topics,
                self.t1_daily_replies,
                self.early_topics,
                self.early_replies,
            ]
            .iter()
            .any(|v| *v < 0)
        {
            Err(ContentError::PolicyPending)
        } else {
            Ok(())
        }
    }
    pub fn authorize(&self, facts: &AuthorityFacts) -> Result<(), ContentError> {
        self.validate()?;
        if !facts.allowed
            || !facts.email_verified
            || !match facts.tier {
                TrustTier::Pending => self.allow_pending,
                TrustTier::T0 => self.allow_t0,
                TrustTier::T1 => self.allow_t1,
            }
        {
            Err(ContentError::Forbidden)
        } else {
            Ok(())
        }
    }
    pub fn validate_text(
        &self,
        title: Option<&str>,
        body: &str,
        is_reply: bool,
        tags: usize,
        external: usize,
        tier: TrustTier,
    ) -> Result<(), ContentError> {
        let (min, max) = if is_reply {
            (self.minimum_reply_chars, self.maximum_reply_chars)
        } else {
            (self.minimum_body_chars, self.maximum_body_chars)
        };
        let count = body.trim().chars().count();
        if count < min
            || count > max
            || tags > self.maximum_tags
            || title.is_some_and(|s| {
                let n = s.trim().chars().count();
                n < self.minimum_title_chars
                    || n > self.maximum_title_chars
                    || s.chars().any(char::is_control)
            })
        {
            return Err(ContentError::InvalidInput);
        }
        let links = if tier == TrustTier::T1 {
            self.max_external_links_t1
        } else {
            self.max_external_links_t0
        };
        if external > links {
            return Err(ContentError::Forbidden);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 批准常量必须与决策记录逐项一致，并通过共用的 `validate`。
    ///
    /// 这条测试的作用是「防误改」：任何人手滑改了某个阈值，会在这里立刻失败，
    /// 而不是等到线上第一次发布才发现规则和 `plan/` 对不上。
    #[test]
    fn approved_rules_match_the_decision_record_and_validate() {
        let r = approved_rules();
        ApprovedContentPolicy::new().unwrap();
        assert_eq!(r.version, APPROVED_RULES_VERSION);
        assert_eq!((r.minimum_title_chars, r.maximum_title_chars), (5, 120));
        assert_eq!((r.minimum_body_chars, r.maximum_body_chars), (10, 50_000));
        assert_eq!((r.minimum_reply_chars, r.maximum_reply_chars), (2, 10_000));
        assert_eq!(r.maximum_tags, 5);
        assert_eq!(r.edit_window_seconds, 1_800);
        assert_eq!(r.delete_window_seconds, 1_800);
        assert_eq!(r.first_post_deletion, FirstPostDeletion::HideTopic);
        assert_eq!((r.t0_daily_topics, r.t0_daily_replies), (5, 20));
        assert_eq!((r.t1_daily_topics, r.t1_daily_replies), (20, 100));
        assert_eq!((r.early_topics, r.early_replies), (1, 5));
        assert_eq!((r.max_external_links_t0, r.max_external_links_t1), (1, 10));
        // DEC-048／DEC-054：配额日界线为 UTC 零点；待审内容同样计入额度，
        // 否则可以靠反复提交待审来绕过每日上限。
        assert_eq!(r.day_utc_offset_minutes, 0);
        assert!(r.count_pending);
        // 邮箱验证是所有等级发布的前置条件：pending 不放行，T0／T1 放行。
        assert!(!r.allow_pending && r.allow_t0 && r.allow_t1);
    }

    /// DEC-055：1 个 Unicode 标量记 1 个字符。
    ///
    /// 十个汉字的正文恰好满足最低 10 字符，说明计数没有按显示宽度做折算；
    /// 同时验证标题上下界是闭区间（5 与 120 都算合法），以及标签数量上限。
    #[test]
    fn length_limits_count_unicode_scalars_without_width_folding() {
        let r = approved_rules();
        let ten_cjk = "草莓社区论坛内容长度";
        assert_eq!(ten_cjk.chars().count(), 10);
        r.validate_text(Some("草莓社区论坛"), ten_cjk, false, 0, 0, TrustTier::T0)
            .unwrap();
        assert!(
            r.validate_text(Some("草莓社区论坛"), "太短了", false, 0, 0, TrustTier::T0)
                .is_err()
        );
        // 标题边界为闭区间：4 个字符拒绝，120 个字符接受，121 个拒绝。
        assert!(
            r.validate_text(Some("四个字符"), ten_cjk, false, 0, 0, TrustTier::T0)
                .is_err()
        );
        r.validate_text(Some(&"标".repeat(120)), ten_cjk, false, 0, 0, TrustTier::T0)
            .unwrap();
        assert!(
            r.validate_text(Some(&"标".repeat(121)), ten_cjk, false, 0, 0, TrustTier::T0)
                .is_err()
        );
        // 回复最低 2 个标量；标签至多 5 个，第 6 个拒绝。
        r.validate_text(None, "行", true, 5, 0, TrustTier::T0).ok();
        r.validate_text(None, "可以", true, 5, 0, TrustTier::T0)
            .unwrap();
        assert!(
            r.validate_text(None, "可以", true, 6, 0, TrustTier::T0)
                .is_err()
        );
    }

    /// DEC-060 外链门槛按信任等级区分：T0 每帖 1 个、T1 每帖 10 个，超出即拒绝。
    #[test]
    fn external_link_thresholds_follow_the_approved_tier_limits() {
        let r = approved_rules();
        let body = "草莓社区论坛内容长度";
        r.validate_text(Some("草莓社区论坛"), body, false, 0, 1, TrustTier::T0)
            .unwrap();
        assert!(
            r.validate_text(Some("草莓社区论坛"), body, false, 0, 2, TrustTier::T0)
                .is_err()
        );
        r.validate_text(Some("草莓社区论坛"), body, false, 0, 10, TrustTier::T1)
            .unwrap();
        assert!(
            r.validate_text(Some("草莓社区论坛"), body, false, 0, 11, TrustTier::T1)
                .is_err()
        );
    }
}
