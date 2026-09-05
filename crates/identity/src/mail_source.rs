//! Reconstruct approved token material only at delivery time, using externally held keys.
//! Queues contain addresses and immutable references, never raw bearer tokens or bodies.
use crate::{
    core::{TokenIssuer, secret_hash},
    repo,
};
use platform::{
    identifiers::SchemaRenderer,
    integrations::AdapterError,
    mail::{MailClass, MailMessage, MailMessageSource},
    outbox::BoxFuture,
};
use platform_db::{Pool, transaction::DbTransaction};
use std::sync::Arc;

pub struct IdentityMailSource {
    pool: Pool,
    renderer: SchemaRenderer,
    issuers: Vec<Arc<TokenIssuer>>,
    from: String,
    base_url: String,
}
impl IdentityMailSource {
    /// Explicit local assembly only in this slice; rotation retains earlier keys for queued work.
    pub fn new_local(
        pool: Pool,
        renderer: SchemaRenderer,
        issuers: Vec<Arc<TokenIssuer>>,
        from: String,
        base_url: String,
    ) -> Result<Self, AdapterError> {
        let authority = base_url
            .strip_prefix("http://")
            .map(|v| v.strip_suffix('/').unwrap_or(v));
        let valid = authority
            .and_then(|v| v.split_once(':'))
            .is_some_and(|(host, port)| {
                matches!(host, "127.0.0.1" | "localhost")
                    && port.bytes().all(|b| b.is_ascii_digit())
                    && port.parse::<u16>().is_ok_and(|p| p > 0)
            });
        if issuers.is_empty() || !valid {
            return Err(AdapterError::InvalidInput);
        }
        let url = base_url.trim_end_matches('/').to_owned();
        Ok(Self {
            pool,
            renderer,
            issuers,
            from,
            base_url: url,
        })
    }
}
impl MailMessageSource for IdentityMailSource {
    fn load<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<MailMessage, AdapterError>> {
        Box::pin(async move {
            if !platform::transaction::validate_operation_id(id) {
                return Err(AdapterError::InvalidInput);
            }
            let mut transaction = DbTransaction::begin_read(&self.pool)
                .await
                .map_err(|_| AdapterError::Unavailable)?;
            let mut tx = transaction.connection();
            let table = repo::table(&tx, &self.renderer, "mail_requests")
                .map_err(|_| AdapterError::Unavailable)?;
            let row=tx.fetch_optional(&format!("SELECT user_id, token_id, recipient_email, recipient_ref, kind, created_at FROM {table} WHERE message_id = ?"),&[repo::text(id)]).await.map_err(|_|AdapterError::Unavailable)?.ok_or(AdapterError::NotFound)?;
            let read = |name: &str| row.text(name).map_err(|_| AdapterError::Unavailable);
            let kind = read("kind")?;
            let token_id = read("token_id")?;
            let user_id = row.i64("user_id").map_err(|_| AdapterError::Unavailable)?;
            let (subject,body)=match kind.as_str(){
            "verify_email"|"reset_password"|"change_email"=>{
                let tokens=repo::table(&tx,&self.renderer,"one_time_tokens").map_err(|_|AdapterError::Unavailable)?;
                let token=tx.fetch_optional(&format!("SELECT user_id, purpose, token_hash, target_email, target_email_key, expires_at, key_version, token_nonce, issued_credential_version FROM {tokens} WHERE token_id = ?"),&[repo::text(&token_id)]).await.map_err(|_|AdapterError::Unavailable)?.ok_or(AdapterError::NotFound)?;
                let get=|name:&str|token.text(name).map_err(|_|AdapterError::Unavailable);
                let issuer=self.issuers.iter().find(|k|k.version==get("key_version").unwrap_or_default()).ok_or(AdapterError::Unavailable)?;
                let expires=token.time("expires_at").map_err(|_|AdapterError::Unavailable)?;let version=token.i64("issued_credential_version").map_err(|_|AdapterError::Unavailable)?;
                // Immutable source reconstruction must survive token consumption and account
                // changes: delivery retry consults durable receipts after loading this body.
                // Token validity is checked exclusively by the consumption transaction.
                if token.i64("user_id").map_err(|_|AdapterError::Unavailable)?!=user_id || get("purpose")?!=kind || get("target_email")?!=read("recipient_email")? {return Err(AdapterError::NotFound)}
                let raw=issuer.derive(&token_id,&get("token_nonce")?,user_id,&kind,&get("target_email_key")?,expires,version);
                if !crate::core::constant_equal(&secret_hash(raw.expose()),&get("token_hash")?){return Err(AdapterError::Unavailable)}
                let (label,path)=match kind.as_str(){"verify_email"=>("验证邮箱","/verify-email"),"reset_password"=>("重置密码","/reset-password"),_=>("确认新邮箱","/confirm-email")};
                (format!("草莓论坛 · {label}"),format!("草莓论坛\n\n请打开 {}{path} 并粘贴以下一次性令牌：\n\n{}\n\n有效期至 {}（UTC）。令牌仅可使用一次。若并非您本人请求，请忽略此邮件。\n",self.base_url,raw.expose(),platform::db::encode_time(expires)))
            },
            "password_changed"=>("草莓论坛 · 密码已变更".into(),"草莓论坛\n\n您的密码已变更，旧登录会话已退出。如非本人操作，请使用找回密码流程。\n".into()),
            "email_changed"=>("草莓论坛 · 邮箱已变更".into(),"草莓论坛\n\n您的账号邮箱已变更，旧登录会话已退出。如非本人操作，请联系站点支持。\n".into()),
            "login_security"=>("草莓论坛 · 账号安全通知".into(),"草莓论坛\n\n您的账号发生了一次登录安全事件。如非本人操作，请重置密码。\n".into()),
            _=>return Err(AdapterError::InvalidInput),
        };
            let message = MailMessage {
                message_id: id.into(),
                recipient_ref: read("recipient_ref")?,
                class: MailClass::Security,
                from: self.from.clone(),
                to: read("recipient_email")?,
                subject,
                text: body,
                unsubscribe_url: None,
                queued_at: row
                    .time("created_at")
                    .map_err(|_| AdapterError::Unavailable)?,
            };
            message.validate()?;
            transaction
                .rollback()
                .await
                .map_err(|_| AdapterError::Unavailable)?;
            Ok(message)
        })
    }
}
