//! DEC-051 identity policy v1. These rules are product defaults, not a claim of full RFC support.
use crate::contract::{IdentityError, IdentityPolicy, InvitationWindow, NormalizedIdentity};
use chrono::{DateTime, Duration, Utc};

pub const IDENTITY_POLICY_VERSION: i64 = 1;
pub const DEFAULT_INVITATION_DAYS: u8 = 3;
pub const MAX_INVITATION_DAYS: u8 = 7;

/// Reserved role/system/brand identifiers also reject ASCII digit substitutions,
/// separators, and role prefixes/suffixes. This list is versioned with this module.
const RESERVED_NAMES: &[&str] = &[
    "admin",
    "administrator",
    "moderator",
    "siteowner",
    "owner",
    "root",
    "system",
    "support",
    "official",
    "staff",
    "security",
    "bbbcm",
    "postmaster",
    "webmaster",
    "abuse",
];

#[derive(Debug, Clone, Copy, Default)]
pub struct ApprovedIdentityPolicy;

fn reserved_skeleton(input: &str) -> String {
    input
        .bytes()
        .filter(|c| *c != b'_')
        .map(|c| match c {
            b'0' => 'o',
            b'1' | b'l' => 'i',
            b'3' => 'e',
            b'4' => 'a',
            b'5' => 's',
            b'7' => 't',
            b'8' => 'b',
            _ => char::from(c),
        })
        .collect()
}
fn reserved(input: &str) -> bool {
    let skeleton = reserved_skeleton(input);
    RESERVED_NAMES.iter().any(|name| {
        let name = reserved_skeleton(name);
        skeleton.starts_with(&name)
            || skeleton.ends_with(&name)
            || input.split('_').any(|part| reserved_skeleton(part) == name)
    })
}
fn plus(now: DateTime<Utc>, duration: Duration) -> Result<DateTime<Utc>, IdentityError> {
    now.checked_add_signed(duration)
        .ok_or(IdentityError::Rejected)
}

impl IdentityPolicy for ApprovedIdentityPolicy {
    fn username(&self, input: &str) -> Result<NormalizedIdentity, IdentityError> {
        let display = input.trim();
        if !(4..=32).contains(&display.len())
            || !display.as_bytes()[0].is_ascii_alphabetic()
            || !display
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(IdentityError::Rejected);
        }
        let key = display.to_ascii_lowercase();
        if reserved(&key) {
            return Err(IdentityError::Rejected);
        }
        Ok(NormalizedIdentity {
            display: display.into(),
            key,
        })
    }

    fn email(&self, input: &str) -> Result<NormalizedIdentity, IdentityError> {
        let input = input.trim();
        if input.len() > 254 || !input.is_ascii() {
            return Err(IdentityError::Rejected);
        }
        let (local, domain) = input.split_once('@').ok_or(IdentityError::Rejected)?;
        if local.is_empty()
            || local.len() > 64
            || local.starts_with('.')
            || local.ends_with('.')
            || local.contains("..")
            || !local
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._%+-".contains(&c))
            || !domain.contains('.')
            || domain
                .rsplit('.')
                .next()
                .is_none_or(|label| !label.bytes().any(|c| c.is_ascii_alphabetic()))
            || !domain.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            return Err(IdentityError::Rejected);
        }
        let display = format!("{local}@{}", domain.to_ascii_lowercase());
        let key = display.to_ascii_lowercase();
        Ok(NormalizedIdentity { display, key })
    }

    fn bio(&self, input: &str) -> Result<String, IdentityError> {
        let value = input.replace("\r\n", "\n");
        if value.chars().count() > 500 || value.chars().any(|c| {
            (c.is_control() && !matches!(c, '\n' | '\t'))
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        }) { return Err(IdentityError::Rejected); }
        Ok(value)
    }

    fn preauth_expires_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        plus(now, Duration::hours(2))
    }
    fn email_change_expires_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        plus(now, Duration::hours(24))
    }
    fn invitation_window(&self, now: DateTime<Utc>) -> Result<InvitationWindow, IdentityError> {
        let start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .ok_or(IdentityError::Rejected)?
            .and_utc();
        Ok(InvitationWindow {
            start,
            end: plus(start, Duration::days(1))?,
            expires_at: plus(now, Duration::days(i64::from(DEFAULT_INVITATION_DAYS)))?,
            daily_limit: 5,
            unused_limit: 5,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_names_fold_case_and_reject_impersonation_and_unicode() {
        let p = ApprovedIdentityPolicy;
        let name = p.username("  Orchard_42  ").unwrap();
        assert_eq!(name.display, "Orchard_42");
        assert_eq!(name.key, "orchard_42");
        assert!(p.username(&"a".repeat(32)).is_ok());
        for name in [
            "",
            "abc",
            "1abc",
            "_abc",
            "a bc",
            "a-bc",
            "a.b c",
            "Ａlice",
            "a\0bc",
            "admin",
            "AdM1N",
            "a_d_m_i_n",
            "admin_orchard",
            "orchard_MODERATOR",
            "orchard_adm1n_member",
            "official42",
            "r00t",
            "bbbcm",
            "b8bcm",
            "bb8cm",
        ] {
            assert!(p.username(name).is_err(), "{name:?}");
        }
        assert!(p.username(&"a".repeat(33)).is_err());
    }
    #[test]
    fn approved_mail_preserves_delivery_case_and_never_merges_provider_aliases() {
        let p = ApprovedIdentityPolicy;
        let email = p.email("  Orchard.Name+Tag@EXAMPLE.INVALID  ").unwrap();
        assert_eq!(email.display, "Orchard.Name+Tag@example.invalid");
        assert_eq!(email.key, "orchard.name+tag@example.invalid");
        assert_ne!(
            email.key,
            p.email("orchardname@example.invalid").unwrap().key
        );
        assert_ne!(
            email.key,
            p.email("orchard.name@example.invalid").unwrap().key
        );
        assert!(
            p.email(&format!("{}@example.invalid", "a".repeat(64)))
                .is_ok()
        );
        for bad in [
            "",
            "a@",
            "@example.invalid",
            "a@@example.invalid",
            "a..b@example.invalid",
            ".ab@example.invalid",
            "ab.@example.invalid",
            "a b@example.invalid",
            "a@example",
            "a@-example.invalid",
            "a@example-.invalid",
            "a@e..invalid",
            "a@127.0.0.1",
            "a@[127.0.0.1]",
            "\"a\"@example.invalid",
            "á@example.invalid",
            "a@例子.invalid",
            "a\r\n@example.invalid",
        ] {
            assert!(p.email(bad).is_err(), "{bad:?}");
        }
        assert!(
            p.email(&format!("{}@example.invalid", "a".repeat(65)))
                .is_err()
        );
        assert!(p.email(&format!("a@{}.invalid", "a".repeat(64))).is_err());
    }
    #[test]
    fn approved_bio_and_lifetimes_have_exact_boundaries() {
        let p = ApprovedIdentityPolicy;
        assert_eq!(p.bio("一行\r\n\t二行").unwrap(), "一行\n\t二行");
        assert!(p.bio("").is_ok());
        assert!(p.bio(&"果".repeat(500)).is_ok());
        for bad in ["x\0y", "x\ry", "x\u{202e}y", "x\u{2066}y"] {
            assert!(p.bio(bad).is_err());
        }
        assert!(p.bio(&"果".repeat(501)).is_err());
        let now = platform::db::decode_time("2026-09-05T23:59:59.000000Z").unwrap();
        assert_eq!(p.preauth_expires_at(now).unwrap(), now + Duration::hours(2));
        assert_eq!(
            p.email_change_expires_at(now).unwrap(),
            now + Duration::hours(24)
        );
        let window = p.invitation_window(now).unwrap();
        assert_eq!(window.expires_at, now + Duration::days(3));
        assert_eq!((window.daily_limit, window.unused_limit), (5, 5));
        assert_eq!(window.end, now + Duration::seconds(1));
        assert_eq!(p.invitation_window(window.end).unwrap().start, window.end);
    }
}
