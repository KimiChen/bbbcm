//! `/manage` 页面的视图模型。
//!
//! 与领域模型分开：这里只放「渲染需要什么」，且**刻意不携带任何 C2/C3 字段**——
//! 证据正文、密钥、令牌都不会进入这些结构，因此模板层不可能不小心把它们渲染出去。
//! 渲染实现在 `apps/web`，本 crate 不依赖具体模板引擎。

/// 一个链接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub label: String,
    pub href: String,
    pub active: bool,
}
impl Link {
    pub fn new(label: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            href: href.into(),
            active: false,
        }
    }
}

/// 表单字段。管理表单一律 SSR 提交，不引入前端脚本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub value: String,
    pub required: bool,
    /// `select` 字段的选项：(取值, 显示名)。其他类型为空。
    ///
    /// 下拉而不是让人手填 ID：填错版块 ID 会把主题移到一个存在但完全无关的版块，
    /// 而这类错误在事后很难被发现。
    pub options: Vec<(String, String)>,
}
impl Field {
    pub fn input(name: &'static str, label: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            label,
            kind,
            value: String::new(),
            required: true,
            options: Vec::new(),
        }
    }
    /// 下拉选择字段。
    pub fn select(name: &'static str, label: &'static str, options: Vec<(String, String)>) -> Self {
        Self {
            name,
            label,
            kind: "select",
            value: String::new(),
            required: true,
            options,
        }
    }
    pub fn hidden(name: &'static str, value: impl Into<String>) -> Self {
        Self {
            name,
            label: "",
            kind: "hidden",
            value: value.into(),
            required: false,
            options: Vec::new(),
        }
    }
}

/// 队列中的一行案件。
///
/// 只带列表需要的字段：证据正文属 C2，连摘要都不放进列表——列表接口顺手带出 C2
/// 是最常见的越权泄露来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseRow {
    pub id: i64,
    pub href: String,
    pub target: String,
    pub status: &'static str,
    pub priority: &'static str,
    pub escalated: bool,
    pub reports: i64,
}

/// 案件详情。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseDetail {
    pub id: i64,
    pub target: String,
    pub status: &'static str,
    pub priority: &'static str,
    pub escalated: bool,
    pub reports: i64,
    /// 证据只给摘要与时间，**不给正文**（C2，`03` §8）。
    pub evidence: Vec<(String, String)>,
    /// 允许推进到的下一状态。由封闭迁移表算出，前端不做判断。
    pub transitions: Vec<Link>,
    /// 本案的举报明细：(类型, 时间, 举报人标识)。
    ///
    /// 举报正文不在这里——它由举报人自由填写，属于案件材料而非队列信息，
    /// 展示它会让审核员在列表页就接触到大量未经处理的用户文本。
    pub report_details: Vec<(String, String, String)>,
}

/// 一个管理页面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub nav: Vec<Link>,
    /// 当前操作者的显示名。未登录时为 `None`。
    pub operator: Option<String>,
    pub csrf: String,
    pub action: String,
    pub submit: &'static str,
    pub fields: Vec<Field>,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub cases: Vec<CaseRow>,
    pub case: Option<CaseDetail>,
    /// 只显示一次的恢复码（`03` §6）。仅在绑定完成的那一次响应里出现。
    pub recovery_codes: Vec<String>,
    /// TOTP 绑定用的 Base32 密钥与 otpauth URI。同样只出现在绑定流程中。
    pub totp: Option<(String, String)>,
}
impl Page {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            nav: Vec::new(),
            operator: None,
            csrf: String::new(),
            action: String::new(),
            submit: "提交",
            fields: Vec::new(),
            notice: None,
            error: None,
            cases: Vec::new(),
            case: None,
            recovery_codes: Vec::new(),
            totp: None,
        }
    }
}

/// 页面渲染器。由 `apps/web` 实现，领域 crate 不依赖模板引擎。
pub trait PageRenderer: Send + Sync {
    fn render(&self, page: &Page) -> Result<String, RenderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderError;
