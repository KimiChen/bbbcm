//! 通用类型：强类型 ID。分页签名 keyset 游标随后续工作包接入。

use serde::de::{Deserializer, Error as DeError, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

/// ID 非法的原因。`01` 第 6 节要求 Rust 拒绝负值。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdError {
    #[error("ID 必须为正整数，收到 {0}")]
    NotPositive(i64),
    #[error("非法 ID 字符串（应为无前导零的十进制正整数）: {0}")]
    Malformed(String),
}

/// 数据库自增 ID 的强类型包装：`Id<Topic>` 与 `Id<User>` 在类型上不能混用。
/// JSON/API 一律以十进制字符串传输（序列化为字符串，反序列化只接受规范十进制字符串）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id<T> {
    raw: i64,
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// 构造并校验：ID 必须为正整数（`01` 第 6 节）。
    pub fn new(raw: i64) -> Result<Self, IdError> {
        if raw <= 0 {
            return Err(IdError::NotPositive(raw));
        }
        Ok(Self {
            raw,
            marker: PhantomData,
        })
    }

    /// 仅供 repo 从数据库回读时使用：主键由数据库生成，已知为正。
    /// 不要用它处理来自请求的输入。
    pub fn from_db(raw: i64) -> Self {
        debug_assert!(raw > 0, "数据库回读的主键应为正数，收到 {raw}");
        Self {
            raw,
            marker: PhantomData,
        }
    }

    pub fn raw(self) -> i64 {
        self.raw
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl<T> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw.to_string())
    }
}

impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdVisitor<T>(PhantomData<fn() -> T>);

        impl<T> Visitor<'_> for IdVisitor<T> {
            type Value = Id<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("十进制字符串形式的 ID")
            }

            fn visit_str<E: DeError>(self, value: &str) -> Result<Id<T>, E> {
                // 只接受规范形式：无符号、无前导零、纯 ASCII 数字。
                // `+1`、`-1`、`007`、`１`（全角）都必须拒绝，否则同一实体会有多种字符串表示。
                if value.is_empty()
                    || value.len() > 19
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || (value.len() > 1 && value.starts_with('0'))
                {
                    return Err(E::custom(IdError::Malformed(value.to_string())));
                }
                let raw: i64 = value
                    .parse()
                    .map_err(|_| E::custom(IdError::Malformed(value.to_string())))?;
                Id::new(raw).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(IdVisitor(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Topic;

    fn parse(s: &str) -> Result<Id<Topic>, serde_json::Error> {
        serde_json::from_str::<Id<Topic>>(&format!("\"{s}\""))
    }

    #[test]
    fn display_is_decimal() {
        let id = Id::<Topic>::new(42).expect("42 是合法 ID");
        assert_eq!(id.to_string(), "42");
        assert_eq!(id.raw(), 42);
    }

    #[test]
    fn rejects_non_positive() {
        assert_eq!(Id::<Topic>::new(0), Err(IdError::NotPositive(0)));
        assert_eq!(Id::<Topic>::new(-1), Err(IdError::NotPositive(-1)));
    }

    #[test]
    fn deserialization_rejects_non_canonical() {
        for bad in [
            "-1",
            "0",
            "+1",
            "007",
            "",
            "1.0",
            "abc",
            "１",
            "9223372036854775808",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} 应被拒绝");
        }
        assert_eq!(parse("42").expect("42 合法").raw(), 42);
    }

    #[test]
    fn serialization_roundtrips_as_string() {
        let id = Id::<Topic>::new(7).expect("7 合法");
        let json = serde_json::to_string(&id).expect("应序列化");
        assert_eq!(json, "\"7\"");
        assert_eq!(parse("7").expect("应反序列化"), id);
    }
}
