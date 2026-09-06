//! Signed fixed-watermark cursors. Keys are application supplied and never logged.
use crate::contract::ContentError;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
#[derive(Clone)]
pub struct CursorKey(Vec<u8>);
impl CursorKey {
    pub fn new(value: Vec<u8>) -> Result<Self, ContentError> {
        if value.len() < 32 {
            Err(ContentError::Unavailable)
        } else {
            Ok(Self(value))
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cursor {
    pub version: u8,
    pub context: String,
    pub watermark: i64,
    pub sort: i64,
    pub id: i64,
}
impl CursorKey {
    pub(crate) fn encode(&self, cursor: &Cursor) -> Result<String, ContentError> {
        let json = serde_json::to_vec(cursor).map_err(|_| ContentError::InvalidCursor)?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.0)
            .map_err(|_| ContentError::InvalidCursor)?;
        mac.update(b"bbbcm.content.cursor.v1\0");
        mac.update(&json);
        Ok(format!(
            "{}.{}",
            hex(&json),
            hex(&mac.finalize().into_bytes())
        ))
    }
    pub(crate) fn decode(&self, text: &str, context: &str) -> Result<Cursor, ContentError> {
        if text.len() > 8192 {
            return Err(ContentError::InvalidCursor);
        }
        let (body, signature) = text.split_once('.').ok_or(ContentError::InvalidCursor)?;
        let bytes = unhex(body)?;
        let signature = unhex(signature)?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.0)
            .map_err(|_| ContentError::InvalidCursor)?;
        mac.update(b"bbbcm.content.cursor.v1\0");
        mac.update(&bytes);
        mac.verify_slice(&signature)
            .map_err(|_| ContentError::InvalidCursor)?;
        let parsed: Cursor =
            serde_json::from_slice(&bytes).map_err(|_| ContentError::InvalidCursor)?;
        if parsed.version != 1
            || parsed.context != context
            || parsed.watermark < 0
            || parsed.sort < 0
            || parsed.id < 0
        {
            return Err(ContentError::InvalidCursor);
        }
        Ok(parsed)
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Result<Vec<u8>, ContentError> {
    if !s.len().is_multiple_of(2)
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ContentError::InvalidCursor);
    }
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| {
            u8::from_str_radix(
                std::str::from_utf8(c).map_err(|_| ContentError::InvalidCursor)?,
                16,
            )
            .map_err(|_| ContentError::InvalidCursor)
        })
        .collect()
}
