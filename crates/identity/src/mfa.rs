//! AUTH-011／AUTH-012 的 TOTP、恢复码与密钥封装。
//!
//! 本模块只做密码学与编码，不访问数据库、不做授权判定：谁能绑定、谁能验签由调用方
//! 在事务内决定。密钥明文只在本模块的函数调用期间存在于内存，绝不返回给上层结构体、
//! 不进日志、不进错误信息（见 `DEC-062`）。
use crate::contract::IdentityError;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// TOTP 参数（RFC 6238）。
///
/// 取 SHA-1、6 位、30 秒步长，是因为主流认证器（尤其 Google Authenticator）实际只支持
/// 这一组；otpauth URI 里的 algorithm 参数会被它们忽略。安全性由密钥长度与服务端的
/// 重放拒绝保证，不靠换算法。
pub const TOTP_DIGITS: u32 = 6;
pub const TOTP_STEP_SECONDS: i64 = 30;
/// 前后各容忍 1 个时间窗，用于吸收客户端时钟漂移。窗口越大越容易被重放，故只取 1。
pub const TOTP_SKEW_STEPS: i64 = 1;
/// 共享密钥字节数。20 字节是 RFC 4226 的推荐下限，且 Base32 后长度对人工输入友好。
pub const TOTP_SECRET_BYTES: usize = 20;
/// 一次生成的恢复码数量（03 §6：只显示一次）。
pub const RECOVERY_CODE_COUNT: usize = 10;

/// 生成一个新的 TOTP 共享密钥。
///
/// 使用操作系统 CSPRNG，与邮件、风险密钥各自独立取材，不复用任何既有密钥材料。
pub fn generate_secret() -> [u8; TOTP_SECRET_BYTES] {
    let mut secret = [0u8; TOTP_SECRET_BYTES];
    OsRng.fill_bytes(&mut secret);
    secret
}

/// RFC 4648 Base32 编码（无填充），用于把密钥放进 otpauth URI 给认证器扫码。
///
/// 手写一小段编码而不是引依赖：这是纯编码不是密码学，且认证器只接受 Base32。
pub fn base32(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for byte in input {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// 计算某个时间步的 TOTP 码（RFC 6238 §4，HMAC-SHA1 + 动态截断）。
///
/// 对 crate 内可见，供绑定流程之外的测试直接构造期望值；不对外暴露，
/// 避免调用方绕过 `verify` 里的时间窗与重放校验自己比对。
pub(crate) fn code_at(secret: &[u8], step: i64) -> Result<u32, IdentityError> {
    let mut mac = <Hmac<sha1::Sha1> as Mac>::new_from_slice(secret)
        .map_err(|_| IdentityError::Unavailable)?;
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    // 动态截断：用最后一个字节的低 4 位作为偏移，取 4 字节并清掉最高位。
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = (u32::from(digest[offset]) & 0x7f) << 24
        | u32::from(digest[offset + 1]) << 16
        | u32::from(digest[offset + 2]) << 8
        | u32::from(digest[offset + 3]);
    Ok(binary % 10u32.pow(TOTP_DIGITS))
}

/// 把 Unix 秒换算为 TOTP 时间步。
pub fn step_for(unix_seconds: i64) -> Result<i64, IdentityError> {
    if unix_seconds < 0 {
        return Err(IdentityError::Rejected);
    }
    Ok(unix_seconds / TOTP_STEP_SECONDS)
}

/// 校验一个 TOTP 码，成功时返回它所属的时间步。
///
/// 调用方必须把返回的时间步与已记录的 `last_step` 比较并落库：同一时间步只允许成功一次，
/// 否则一个被旁观者看到的验证码可以在 30 秒内被重放。容差窗口内的比较使用常量时间等价的
/// 逐窗口比较——每个窗口都算完再比，不因先命中而提前返回。
pub fn verify(secret: &[u8], candidate: &str, now_step: i64) -> Result<i64, IdentityError> {
    if candidate.len() != TOTP_DIGITS as usize || !candidate.bytes().all(|b| b.is_ascii_digit()) {
        return Err(IdentityError::Rejected);
    }
    let expected: u32 = candidate.parse().map_err(|_| IdentityError::Rejected)?;
    let mut matched: Option<i64> = None;
    for delta in -TOTP_SKEW_STEPS..=TOTP_SKEW_STEPS {
        let step = now_step.checked_add(delta).ok_or(IdentityError::Rejected)?;
        if step < 0 {
            continue;
        }
        if code_at(secret, step)? == expected {
            matched = Some(step);
        }
    }
    matched.ok_or(IdentityError::Rejected)
}

/// 用途隔离的密钥封装（DEC-062）。
///
/// 包裹密钥由外部注入的版本化 keyring 派生，与邮件、风险密钥的用途标签不同，
/// 因此任何一处密钥泄露都不会横向解开另一处。
pub struct SecretWrapper {
    key: [u8; 32],
    version: i64,
}
impl std::fmt::Debug for SecretWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretWrapper([REDACTED])")
    }
}
impl SecretWrapper {
    /// 从 keyring 的某个版本派生包裹密钥。
    ///
    /// 输入材料长度不足直接失败关闭，不做补零或拉伸——弱密钥比没有密钥更危险，
    /// 因为它会让人误以为数据已被保护。
    pub fn new(material: &[u8], version: i64) -> Result<Self, IdentityError> {
        if material.len() < 32 || version <= 0 {
            return Err(IdentityError::Rejected);
        }
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(material)
            .map_err(|_| IdentityError::Unavailable)?;
        mac.update(b"identity.mfa.totp.secret.v1");
        let derived = mac.finalize().into_bytes();
        let mut key = [0u8; 32];
        key.copy_from_slice(&derived);
        Ok(Self { key, version })
    }
    pub fn version(&self) -> i64 {
        self.version
    }
    /// 加密共享密钥，返回（密文十六进制, nonce 十六进制）。
    ///
    /// 每次加密都新取随机 nonce：XChaCha20 的 192 位 nonce 使随机取值的碰撞概率
    /// 可以忽略，因此不需要维护计数器状态。
    pub fn seal(&self, secret: &[u8]) -> Result<(String, String), IdentityError> {
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let sealed = cipher
            .encrypt(XNonce::from_slice(&nonce), secret)
            .map_err(|_| IdentityError::Unavailable)?;
        Ok((hex(&sealed), hex(&nonce)))
    }
    /// 解出共享密钥。任何解密失败一律返回 `Rejected`，不区分「nonce 坏了」与
    /// 「密文被篡改」——区分只会给攻击者提供预言机。
    pub fn open(&self, cipher_hex: &str, nonce_hex: &str) -> Result<Vec<u8>, IdentityError> {
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        let nonce = unhex(nonce_hex)?;
        if nonce.len() != 24 {
            return Err(IdentityError::Rejected);
        }
        cipher
            .decrypt(XNonce::from_slice(&nonce), unhex(cipher_hex)?.as_slice())
            .map_err(|_| IdentityError::Rejected)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>, IdentityError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(IdentityError::Rejected);
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| IdentityError::Rejected))
        .collect()
}

/// 生成一组一次性恢复码。
///
/// 返回明文供「只显示一次」的界面使用；调用方只能把哈希落库（03 §6）。
/// 用 Base32 且分组显示，便于人工抄写又不引入易混淆字符。
pub fn generate_recovery_codes() -> Vec<String> {
    (0..RECOVERY_CODE_COUNT)
        .map(|_| {
            let mut raw = [0u8; 10];
            OsRng.fill_bytes(&mut raw);
            let encoded = base32(&raw);
            format!("{}-{}", &encoded[..8], &encoded[8..16])
        })
        .collect()
}

#[cfg(test)]
mod tests;
