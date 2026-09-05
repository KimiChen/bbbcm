//! SSR projections and renderer injection; no database rows cross this boundary.
use crate::contract::IdentityError;

#[derive(Default)]
pub struct PageFrame {
    pub flash: Option<&'static str>,
    pub error: Option<&'static str>,
    pub empty: Option<&'static str>,
}
pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub value: String,
    pub required: bool,
}
impl Field {
    pub fn input(name: &'static str, label: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            label,
            kind,
            value: String::new(),
            required: true,
        }
    }
    pub fn hidden(name: &'static str, value: impl Into<String>) -> Self {
        Self {
            name,
            label: "",
            kind: "hidden",
            value: value.into(),
            required: false,
        }
    }
}
pub struct Row {
    pub label: String,
    pub value: String,
    pub action: Option<&'static str>,
    pub action_label: &'static str,
    pub reference: String,
}
pub struct Page {
    pub frame: PageFrame,
    pub title: &'static str,
    pub action: &'static str,
    pub submit: &'static str,
    pub csrf: String,
    pub idempotency_key: String,
    pub fields: Vec<Field>,
    pub rows: Vec<Row>,
    pub account_page: bool,
    pub show_form: bool,
    pub policy_consent: bool,
    pub preset_avatar: bool,
}
impl Page {
    pub fn new(title: &'static str, action: &'static str) -> Self {
        Self {
            frame: PageFrame::default(),
            title,
            action,
            submit: "提交",
            csrf: String::new(),
            idempotency_key: platform::transaction::new_operation_id(),
            fields: Vec::new(),
            rows: Vec::new(),
            account_page: action.starts_with("/settings") || action == "/me/invites",
            show_form: true,
            policy_consent: false,
            preset_avatar: false,
        }
    }
}
/// Applications compile Askama with their existing local asset resolver.
pub trait PageRenderer: Send + Sync {
    fn render(&self, page: &Page) -> Result<String, IdentityError>;
}
