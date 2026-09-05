//! Local MIME spool and explicitly simulated feedback. No SMTP or DNS access.
use super::*;
use crate::integrations::local::LocalRoot;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Clone)]
pub struct LocalMailProvider {
    pub root: LocalRoot,
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
/// The local preview uses A00's approved project brand asset. Reference-site mail
/// markup was not publicly observed; this does not claim to reproduce it.
pub fn render_local_message(
    event_id: &str,
    message: &MailMessage,
) -> Result<Vec<u8>, AdapterError> {
    message.validate()?;
    if !crate::transaction::validate_operation_id(event_id) {
        return Err(AdapterError::InvalidInput);
    }
    let boundary = format!(
        "bcm-{}",
        crate::integrations::local::digest(event_id.as_bytes())
    );
    if message.text.contains(&boundary) {
        return Err(AdapterError::InvalidInput);
    }
    let alternative = format!("{boundary}-alternative");
    let mut header = format!(
        "From: 草莓论坛 <{}>\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}@local.test>\r\nMIME-Version: 1.0\r\nX-BBBcm-Delivery: local-test-only\r\nX-BBBcm-Class: {}\r\n",
        message.from,
        message.to,
        message.subject,
        message.queued_at.to_rfc2822(),
        event_id,
        message.class.as_str()
    );
    if message.class == MailClass::Marketing {
        header.push_str(&format!(
            "List-Unsubscribe: <{}>\r\n",
            message
                .unsubscribe_url
                .as_deref()
                .ok_or(AdapterError::InvalidInput)?
        ));
        // RFC8058 additionally requires a valid DKIM signature over these headers.
        // No one-click declaration is emitted while that production path is disabled.
    } else {
        header.push_str("Auto-Submitted: auto-generated\r\n");
    }
    let body = escape_html(&message.text).replace('\n', "<br>\n");
    let unsubscribe = message
        .unsubscribe_url
        .as_ref()
        .map(|url| format!("<p><a href=\"{}\">退订营销邮件</a></p>", escape_html(url)))
        .unwrap_or_default();
    let brand = include_str!("../../../../apps/web/static/img/email-header.svg");
    header.push_str(&format!("Content-Type: multipart/related; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: multipart/alternative; boundary=\"{alternative}\"\r\n\r\n--{alternative}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n草莓论坛\r\n本地测试邮件：未进行真实投递。\r\n{}\r\n--{alternative}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n<!doctype html><html lang=\"zh-CN\"><body><img src=\"cid:brand-header\" alt=\"草莓论坛\" width=\"640\" height=\"160\"><p>本地测试邮件：未进行真实投递。</p><p>{body}</p>{unsubscribe}</body></html>\r\n--{alternative}--\r\n--{boundary}\r\nContent-Type: image/svg+xml; charset=utf-8\r\nContent-Disposition: inline\r\nContent-ID: <brand-header>\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{brand}\r\n--{boundary}--\r\n", message.text.replace("\r\n", "\n").replace('\n', "\r\n")));
    Ok(header.into_bytes())
}
impl MailProvider for LocalMailProvider {
    fn simulated(&self) -> bool {
        true
    }
    fn submit<'a>(
        &'a self,
        event_id: &'a str,
        message: &'a MailMessage,
    ) -> BoxFuture<'a, Result<DeliveryReceipt, AdapterError>> {
        Box::pin(async move {
            self.root
                .write(
                    "mail",
                    event_id,
                    "eml",
                    render_local_message(event_id, message)?,
                )
                .await?;
            Ok(DeliveryReceipt {
                state: DeliveryState::LocalRecorded,
                simulated: true,
            })
        })
    }
}

/// Test source is constructed only by local application wiring; it cannot pretend
/// to be an authenticated provider or report real delivery measurements.
pub struct LocalFeedbackSource(Mutex<VecDeque<TrustedFeedback>>);
impl LocalFeedbackSource {
    pub fn new(
        records: Vec<TrustedFeedback>,
        environment: crate::config::Environment,
    ) -> Result<Self, AdapterError> {
        if environment.is_production_like()
            || records
                .iter()
                .any(|f| !f.simulated || f.provider != "local_test" || f.validate().is_err())
        {
            return Err(AdapterError::InvalidInput);
        }
        Ok(Self(Mutex::new(records.into())))
    }
}
impl FeedbackSource for LocalFeedbackSource {
    fn next<'a>(&'a self) -> BoxFuture<'a, Result<Option<TrustedFeedback>, AdapterError>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .map_err(|_| AdapterError::Unavailable)?
                .front()
                .cloned())
        })
    }
    fn acknowledge<'a>(
        &'a self,
        feedback: &'a TrustedFeedback,
    ) -> BoxFuture<'a, Result<(), AdapterError>> {
        Box::pin(async move {
            let mut records = self.0.lock().map_err(|_| AdapterError::Unavailable)?;
            match records.front() {
                Some(record) if record.fingerprint() == feedback.fingerprint() => {
                    records.pop_front();
                    Ok(())
                }
                _ => Err(AdapterError::Conflict),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn message(class: MailClass) -> MailMessage {
        MailMessage {
            message_id: crate::transaction::new_operation_id(),
            recipient_ref: crate::transaction::new_operation_id(),
            class,
            from: "noreply@example.test".into(),
            to: "synthetic@example.test".into(),
            subject: "测试通知".into(),
            text: "合成正文 <script>alert(1)</script>".into(),
            unsubscribe_url: (class == MailClass::Marketing)
                .then(|| "https://example.test/unsubscribe/synthetic".into()),
            queued_at: crate::db::decode_time("2026-09-05T00:00:00Z").unwrap(),
        }
    }
    #[test]
    fn mail_classification_headers_brand_and_sensitive_debug() {
        for class in [
            MailClass::Security,
            MailClass::Transactional,
            MailClass::Marketing,
        ] {
            let mail = message(class);
            let id = crate::transaction::new_operation_id();
            let mime = String::from_utf8(render_local_message(&id, &mail).unwrap()).unwrap();
            assert!(mime.contains("草莓论坛"));
            assert!(mime.contains("cid:brand-header"));
            assert!(mime.contains("&lt;script&gt;"));
            assert_eq!(
                mime.contains("List-Unsubscribe:"),
                class == MailClass::Marketing
            );
            assert_eq!(
                mime.contains("Auto-Submitted: auto-generated"),
                class != MailClass::Marketing
            );
            assert!(!mime.contains("List-Unsubscribe-Post:"));
            assert!(!format!("{mail:?}").contains("synthetic@example.test"));
            assert!(!format!("{mail:?}").contains("合成正文"));
        }
        let mut mail = message(MailClass::Security);
        mail.subject = "test\r\nBcc: other@example.test".into();
        assert_eq!(mail.validate(), Err(AdapterError::InvalidInput));
        mail = message(MailClass::Marketing);
        mail.unsubscribe_url = None;
        assert_eq!(mail.validate(), Err(AdapterError::InvalidInput));
        mail = message(MailClass::Marketing);
        mail.unsubscribe_url = Some("https://secret@example.test/u".into());
        assert_eq!(mail.validate(), Err(AdapterError::InvalidInput));
    }
    #[test]
    fn no_guessed_alert_thresholds() {
        let config = MailAlertConfig::default();
        assert!(
            config
                .evaluate(&MailMetricSample {
                    submitted: 3,
                    bounced: 2,
                    ..Default::default()
                })
                .unwrap()
                .is_empty()
        );
        let config = MailAlertConfig {
            min_samples: Some(10),
            bounce_rate: Some(0.1),
            complaint_rate: Some(0.01),
            delivery_delay_ms: Some(1000),
        };
        assert_eq!(
            config
                .evaluate(&MailMetricSample {
                    submitted: 10,
                    bounced: 2,
                    complained: 1,
                    max_delay_ms: Some(1001),
                    ..Default::default()
                })
                .unwrap(),
            vec![
                MailMetricAlert::Delay,
                MailMetricAlert::BounceRate,
                MailMetricAlert::ComplaintRate
            ]
        );
        assert!(
            MailAlertConfig {
                bounce_rate: Some(0.1),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
