use super::*;

/// RFC 6238 附录 B 的官方测试向量（HMAC-SHA1，8 位）。
///
/// 我们生产用 6 位，因此断言取官方 8 位结果的后 6 位——这正是 RFC 定义的截断关系。
/// 用官方向量而不是自造期望值，才能证明实现与标准一致，而不是「与自己一致」。
#[test]
fn totp_matches_the_rfc6238_reference_vectors() {
    let secret = b"12345678901234567890";
    for (unix, expected8) in [
        (59_i64, "94287082"),
        (1_111_111_109, "07081804"),
        (1_111_111_111, "14050471"),
        (1_234_567_890, "89005924"),
        (2_000_000_000, "69279037"),
    ] {
        let step = step_for(unix).unwrap();
        let code = format!("{:06}", code_at(secret, step).unwrap());
        assert_eq!(code, expected8[2..], "unix={unix}");
        // 同一个码在它自己的时间步上必须验签通过，并返回该时间步。
        assert_eq!(verify(secret, &code, step).unwrap(), step);
    }
}

/// 时间窗容差：前后各 1 步接受，第 2 步一律拒绝。
#[test]
fn totp_accepts_one_step_of_drift_and_rejects_more() {
    let secret = generate_secret();
    let step = step_for(1_700_000_000).unwrap();
    let code = format!("{:06}", code_at(&secret, step).unwrap());
    for delta in [-1, 0, 1] {
        assert_eq!(
            verify(&secret, &code, step - delta).unwrap(),
            step,
            "delta={delta} 应在容差内"
        );
    }
    for delta in [-2, 2, 100] {
        assert!(
            verify(&secret, &code, step - delta).is_err(),
            "delta={delta} 应超出容差"
        );
    }
    // 位数不对、含非数字一律拒绝，不进入 HMAC 计算。
    for bad in ["12345", "1234567", "12345a", ""] {
        assert!(verify(&secret, bad, step).is_err());
    }
}

/// 密钥封装：密文不含明文、能解回原值、篡改任一部分都失败。
#[test]
fn wrapped_secret_round_trips_and_rejects_tampering() {
    let wrapper = SecretWrapper::new(&[7u8; 32], 3).unwrap();
    assert_eq!(wrapper.version(), 3);
    let secret = generate_secret();
    let (cipher, nonce) = wrapper.seal(&secret).unwrap();
    assert_eq!(wrapper.open(&cipher, &nonce).unwrap(), secret.to_vec());
    // 密文里不得出现明文的十六进制形式。
    let plain_hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    assert!(!cipher.contains(&plain_hex));
    // 每次加密取新 nonce，因此同一密钥两次封装的密文必然不同。
    let (cipher2, nonce2) = wrapper.seal(&secret).unwrap();
    assert_ne!(nonce, nonce2);
    assert_ne!(cipher, cipher2);
    // 篡改密文、换 nonce、换密钥版本材料，一律解不开。
    let mut tampered: Vec<char> = cipher.chars().collect();
    tampered[0] = if tampered[0] == 'a' { 'b' } else { 'a' };
    let tampered: String = tampered.into_iter().collect();
    assert!(wrapper.open(&tampered, &nonce).is_err());
    assert!(wrapper.open(&cipher, &nonce2).is_err());
    let other = SecretWrapper::new(&[9u8; 32], 3).unwrap();
    assert!(other.open(&cipher, &nonce).is_err());
    // 非法十六进制与错误长度的 nonce 直接拒绝。
    assert!(wrapper.open("zz", &nonce).is_err());
    assert!(wrapper.open(&cipher, "00").is_err());
    // 密钥材料太短一律拒绝，不做补零或拉伸。
    assert!(SecretWrapper::new(&[1u8; 31], 1).is_err());
    assert!(SecretWrapper::new(&[1u8; 32], 0).is_err());
    // Debug 输出不得泄露密钥材料。
    assert_eq!(format!("{wrapper:?}"), "SecretWrapper([REDACTED])");
}

/// 恢复码与 Base32 编码。
#[test]
fn recovery_codes_are_distinct_and_base32_matches_rfc4648() {
    // RFC 4648 §10 的标准测试向量，证明编码实现正确（认证器靠它读密钥）。
    assert_eq!(base32(b""), "");
    assert_eq!(base32(b"f"), "MY");
    assert_eq!(base32(b"fo"), "MZXQ");
    assert_eq!(base32(b"foo"), "MZXW6");
    assert_eq!(base32(b"foob"), "MZXW6YQ");
    assert_eq!(base32(b"fooba"), "MZXW6YTB");
    assert_eq!(base32(b"foobar"), "MZXW6YTBOI");

    let codes = generate_recovery_codes();
    assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
    // 每个码互不相同；重复会让「单次消费」变成可被绕过的假象。
    let unique: std::collections::BTreeSet<_> = codes.iter().collect();
    assert_eq!(unique.len(), codes.len());
    for code in &codes {
        assert_eq!(code.len(), 17, "8-8 分组加一个连字符");
        assert!(
            code.chars()
                .all(|c| c == '-' || c.is_ascii_uppercase() || c.is_ascii_digit())
        );
    }
}
