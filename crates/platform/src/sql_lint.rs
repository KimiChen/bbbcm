//! DEC-033 / 02 §6.1 的共享 SQL 门禁。
//!
//! Repository 必须保留完整 SQL 字面量（可带受信标识符 `{table}` 占位），
//! 不支持把 SQL 拆成片段再拼接。这里做词法与有限结构检查，不冒充数据库解析器；
//! 数据类型、NOT NULL、绑定来源和 SchemaRenderer 的实际使用仍须设计与代码复核。

use std::fmt;

/// 错误只描述规则和位置，不复制 SQL 内容或潜在 Secret。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintError {
    pub offset: usize,
    pub rule: &'static str,
}

impl fmt::Display for LintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SQL lint: {}（字节 {}）", self.rule, self.offset)
    }
}

impl std::error::Error for LintError {}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    offset: usize,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Literal,
    Symbol,
    Hole,
}

impl Token {
    fn is(&self, word: &str) -> bool {
        matches!(self.kind, Kind::Word | Kind::Symbol) && self.text.eq_ignore_ascii_case(word)
    }
}

fn error(offset: usize, rule: &'static str) -> LintError {
    LintError { offset, rule }
}

/// 验证 Repository 完整 SQL 模板；普通数据值一律通过 `?` 绑定。
pub fn lint_sql(sql: &str) -> Result<(), LintError> {
    let tokens = sql_tokens(sql)?;
    lint_global_tokens(&tokens)?;
    let depths = depths(&tokens)?;
    for statement in tokens.split(|token| token.is(";")) {
        if let Some(first) = statement.first()
            && !["SELECT", "INSERT", "UPDATE", "DELETE", "WITH"]
                .iter()
                .any(|word| first.is(word))
        {
            return Err(error(
                first.offset,
                "Repository 仅允许完整 SELECT/INSERT/UPDATE/DELETE/WITH 语句",
            ));
        }
    }
    for (i, token) in tokens.iter().enumerate() {
        let rule = if token.kind == Kind::Hole && !identifier(&token.text) {
            Some("共享 SQL 模板只允许命名标识符占位")
        } else if token.kind == Kind::Hole
            && !i
                .checked_sub(1)
                .and_then(|n| tokens.get(n))
                .is_some_and(|previous| {
                    ["FROM", "JOIN", "INTO", "UPDATE"]
                        .iter()
                        .any(|word| previous.is(word))
                })
        {
            Some("SQL 模板占位只允许表位置；值必须 bind，禁止动态SQL片段")
        } else if token.is("RETURNING") {
            Some("禁止 RETURNING")
        } else if sequence(&tokens, i, &["ON", "CONFLICT"])
            || sequence(&tokens, i, &["ON", "DUPLICATE", "KEY"])
        {
            Some("upsert 必须走平台抽象")
        } else if ["ILIKE", "CAST"].iter().any(|word| token.is(word))
            || sequence(&tokens, i, &["FOR", "UPDATE"])
            || sequence(&tokens, i, &["FOR", "NO", "KEY", "UPDATE"])
            || sequence(&tokens, i, &["FOR", "SHARE"])
            || sequence(&tokens, i, &["FOR", "KEY", "SHARE"])
            || sequence(&tokens, i, &["SKIP", "LOCKED"])
            || token.is("::")
        {
            Some("禁止共享 SQL 方言或数据库 CAST")
        } else if token.is("\"") || token.is("`") || token.is("[") {
            Some("禁止引号标识符；使用 SchemaRenderer")
        } else if token.is("||") {
            Some("字符串拼接必须在应用层完成")
        } else if sequence(&tokens, i, &["NULLS", "FIRST"])
            || sequence(&tokens, i, &["NULLS", "LAST"])
        {
            Some("排序列必须非空，禁止 NULLS FIRST/LAST")
        } else if sequence(&tokens, i, &["IS", "DISTINCT", "FROM"])
            || sequence(&tokens, i, &["IS", "NOT", "DISTINCT", "FROM"])
        {
            Some("禁止 IS DISTINCT FROM")
        } else if token.is("$") || token.is(":") || token.is("@") {
            Some("查询值必须使用 ? 占位符")
        } else if token.kind == Kind::Word && token.text.to_ascii_lowercase().starts_with("bcm_") {
            Some("禁止 Repository 硬编码物理前缀")
        } else if is_function(&tokens, i)
            && [
                "LASTVAL",
                "CURRVAL",
                "PG_GET_SERIAL_SEQUENCE",
                "LAST_INSERT_ROWID",
                "LAST_INSERT_ID",
            ]
            .iter()
            .any(|word| token.is(word))
        {
            Some("主键回读必须走平台抽象")
        } else {
            None
        };
        if let Some(rule) = rule {
            return Err(error(token.offset, rule));
        }
        if token.is("UPDATE") && !tokens.get(i.wrapping_sub(1)).is_some_and(|t| t.is("FOR")) {
            check_update(&tokens, &depths, i)?;
        }
        if token.is("DELETE") {
            check_delete(&tokens, &depths, i)?;
        }
        if token.is("IN") && tokens.get(i + 1).is_some_and(|t| t.is("(")) {
            let end = closing_paren(&tokens, i + 1)?;
            let body = &tokens[i + 2..end];
            if body.iter().any(|t| t.is("SELECT") || t.is("WITH"))
                && body.iter().any(|t| t.is("LIMIT"))
            {
                return Err(error(token.offset, "禁止 IN 子查询内含 LIMIT"));
            }
        }
        if token.is("LIKE") {
            check_like(&tokens, i)?;
        }
    }
    Ok(())
}

/// 方言迁移仅检查全局禁令；不会把 PostgreSQL ACL/角色语法误判为共享 SQL。
pub fn lint_migration_sql(sql: &str) -> Result<(), LintError> {
    lint_global_tokens(&sql_tokens(sql)?)
}

fn lint_global_tokens(tokens: &[Token]) -> Result<(), LintError> {
    for (i, token) in tokens.iter().enumerate() {
        if is_function(tokens, i)
            && ([
                "GEN_RANDOM_UUID",
                "GEN_RANDOM_BYTES",
                "UUID",
                "UUID_SHORT",
                "RANDOM_UUID",
                "RANDOMBLOB",
                "RANDOM_BYTES",
                "RANDOM",
                "RAND",
                "NEWID",
                "NEWSEQUENTIALID",
            ]
            .iter()
            .any(|word| token.is(word))
                || token
                    .text
                    .to_ascii_uppercase()
                    .starts_with("UUID_GENERATE_"))
        {
            return Err(error(token.offset, "禁止数据库 UUID/随机 ID 函数"));
        }
        if token.is("CREATE") || token.is("ALTER") || token.is("DROP") {
            let object = tokens[i + 1..].iter().find(|t| {
                ![
                    "OR",
                    "REPLACE",
                    "TEMP",
                    "TEMPORARY",
                    "MATERIALIZED",
                    "UNIQUE",
                    "IF",
                    "NOT",
                    "EXISTS",
                ]
                .iter()
                .any(|w| t.is(w))
            });
            if object.is_some_and(|t| {
                ["TRIGGER", "PROCEDURE", "FUNCTION", "VIEW"]
                    .iter()
                    .any(|word| t.is(word))
            }) {
                return Err(error(
                    token.offset,
                    "禁止触发器、存储过程、数据库函数和视图",
                ));
            }
            if token.is("CREATE") && object.is_some_and(|t| t.is("INDEX")) {
                let statement = tokens[i + 1..].iter().take_while(|t| !t.is(";"));
                if statement.into_iter().any(|t| t.is("WHERE")) {
                    return Err(error(token.offset, "禁止部分索引"));
                }
            }
        }
    }
    Ok(())
}

fn sequence(tokens: &[Token], start: usize, words: &[&str]) -> bool {
    words
        .iter()
        .enumerate()
        .all(|(n, word)| tokens.get(start + n).is_some_and(|t| t.is(word)))
}

fn is_function(tokens: &[Token], i: usize) -> bool {
    tokens[i].kind == Kind::Word && tokens.get(i + 1).is_some_and(|t| t.is("("))
}

fn depths(tokens: &[Token]) -> Result<Vec<usize>, LintError> {
    let mut depth = 0usize;
    let mut result = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token.is(")") {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| error(token.offset, "SQL 括号不匹配"))?;
        }
        result.push(depth);
        if token.is("(") {
            depth += 1;
        }
    }
    if depth != 0 {
        return Err(error(
            tokens.last().map_or(0, |t| t.offset),
            "SQL 括号不匹配",
        ));
    }
    Ok(result)
}

fn closing_paren(tokens: &[Token], start: usize) -> Result<usize, LintError> {
    if !tokens.get(start).is_some_and(|token| token.is("(")) {
        return Err(error(
            tokens.get(start).map_or(0, |token| token.offset),
            "仅支持圆括号参数列表",
        ));
    }
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(start) {
        if token.is("(") {
            depth += 1;
        } else if token.is(")") {
            depth -= 1;
            if depth == 0 {
                return Ok(i);
            }
        }
    }
    Err(error(tokens[start].offset, "SQL 括号不匹配"))
}

fn same_level<'a>(tokens: &'a [Token], depths: &[usize], start: usize) -> Vec<&'a Token> {
    tokens
        .iter()
        .enumerate()
        .skip(start + 1)
        .take_while(|(i, token)| depths[*i] >= depths[start] && !token.is(";"))
        .filter(|(i, _)| depths[*i] == depths[start])
        .map(|(_, token)| token)
        .collect()
}

fn check_update(tokens: &[Token], depths: &[usize], i: usize) -> Result<(), LintError> {
    let tail = same_level(tokens, depths, i);
    let mut before_set = true;
    for token in tail {
        if token.is("SET") {
            before_set = false;
        }
        if token.is("FROM") || token.is("JOIN") || (before_set && token.is(",")) {
            return Err(error(token.offset, "禁止 UPDATE FROM/JOIN 或多表 UPDATE"));
        }
    }
    Ok(())
}

fn check_delete(tokens: &[Token], depths: &[usize], i: usize) -> Result<(), LintError> {
    let tail = same_level(tokens, depths, i);
    if !tail.first().is_some_and(|t| t.is("FROM")) {
        return Err(error(tokens[i].offset, "禁止多表 DELETE"));
    }
    for token in tail.iter().skip(1).take_while(|t| !t.is("WHERE")) {
        if token.is("JOIN") || token.is("USING") || token.is(",") {
            return Err(error(token.offset, "禁止多表 DELETE"));
        }
    }
    Ok(())
}

fn check_like(tokens: &[Token], i: usize) -> Result<(), LintError> {
    let mut lhs = i.checked_sub(1).and_then(|n| tokens.get(n));
    if lhs.is_some_and(|t| t.is("NOT")) {
        lhs = i.checked_sub(2).and_then(|n| tokens.get(n));
    }
    if !lhs.is_some_and(|t| {
        t.kind == Kind::Word && t.text.to_ascii_lowercase().ends_with("_normalized")
    }) {
        return Err(error(
            tokens[i].offset,
            "LIKE 仅允许应用层 _normalized 规范化列",
        ));
    }
    // 将模式限制为绑定值或字面量，使 ESCAPE 必须属于本次 LIKE，而非后续表达式。
    if !tokens
        .get(i + 1)
        .is_some_and(|t| t.is("?") || t.kind == Kind::Literal)
        || !tokens.get(i + 2).is_some_and(|t| t.is("ESCAPE"))
        || !tokens
            .get(i + 3)
            .is_some_and(|t| t.kind == Kind::Literal && t.text.chars().count() == 1)
    {
        return Err(error(tokens[i].offset, "LIKE 必须带单字符字面量 ESCAPE"));
    }
    Ok(())
}

fn sql_tokens(sql: &str) -> Result<Vec<Token>, LintError> {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < sql.len() {
        let rest = &sql[i..];
        let ch = rest.chars().next().expect("非空");
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        if rest.starts_with("--") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            i = block_comment_end(sql, i, false)?;
            continue;
        }
        let offset = i;
        let (text, kind) = if ch == '\'' {
            if tokens
                .last()
                .is_some_and(|t: &Token| t.is("E") && t.offset + 1 == offset)
            {
                return Err(error(offset, "共享 SQL 与迁移禁止 PostgreSQL E 字符串"));
            }
            let mut literal = String::new();
            i += 1;
            loop {
                let next = sql[i..]
                    .chars()
                    .next()
                    .ok_or_else(|| error(offset, "SQL 字符串未闭合"))?;
                i += next.len_utf8();
                if next == '\'' {
                    if sql[i..].starts_with('\'') {
                        literal.push('\'');
                        i += 1;
                    } else {
                        break;
                    }
                } else if next == '\\' {
                    return Err(error(
                        i - 1,
                        "SQL 字面量禁止反斜杠转义；数据用 bind，LIKE ESCAPE 使用 ! 等字符",
                    ));
                } else {
                    literal.push(next);
                }
            }
            (literal, Kind::Literal)
        } else if rest.starts_with("{{") {
            // 迁移执行器的受信标识符模板；共享 SQL 不允许这类方言模板。
            let end = rest
                .find("}}")
                .ok_or_else(|| error(i, "迁移标识符占位未闭合"))?;
            i += end + 2;
            (rest[2..end].to_owned(), Kind::Hole)
        } else if ch == '{' {
            let end = rest
                .find('}')
                .ok_or_else(|| error(i, "SQL 标识符占位未闭合"))?;
            let name = &rest[1..end];
            if !identifier(name) {
                return Err(error(i, "SQL 模板只允许命名标识符占位"));
            }
            i += end + 1;
            (name.to_owned(), Kind::Hole)
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            i += rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            (sql[offset..i].to_ascii_uppercase(), Kind::Word)
        } else if rest.starts_with("::") || rest.starts_with("||") {
            i += 2;
            (sql[offset..i].to_owned(), Kind::Symbol)
        } else {
            i += ch.len_utf8();
            (ch.to_string(), Kind::Symbol)
        };
        tokens.push(Token { text, offset, kind });
    }
    Ok(tokens)
}

fn block_comment_end(source: &str, start: usize, allow_nested: bool) -> Result<usize, LintError> {
    let bytes = source.as_bytes();
    let mut i = start + 2;
    let mut depth = 1usize;
    while i + 1 < bytes.len() {
        if &bytes[i..i + 2] == b"/*" {
            if !allow_nested {
                return Err(error(i, "共享 SQL 与迁移禁止嵌套块注释（三库解析不一致）"));
            }
            depth += 1;
            i += 2;
        } else if &bytes[i..i + 2] == b"*/" {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Ok(i);
            }
        } else {
            i += 1;
        }
    }
    Err(error(start, "块注释未闭合"))
}

/// 扫描 Repository Rust 源文件；注释与非 SQL 提示文本不会作为 SQL 扫描。
///
/// 不允许 repo 中 concat!/include_str!、字符串加法或 push_str/join/extend 拼接。
/// 这是有意采用的失败关闭边界：新增动态 SQL 必须先集中到平台方言模块或保留完整模板。
pub fn lint_repo_source(source: &str) -> Result<(), LintError> {
    let tokens = rust_tokens(source)?;
    for (i, token) in tokens.iter().enumerate() {
        if token.kind == Kind::Literal && is_sql(&token.text) {
            lint_sql(&token.text).map_err(|e| error(token.offset + e.offset, e.rule))?;
        }
        if token.kind == Kind::Word
            && tokens.get(i + 1).is_some_and(|t| t.is("!"))
            && [
                "concat",
                "include_str",
                "include_bytes",
                "query",
                "query_as",
                "query_scalar",
            ]
            .iter()
            .any(|name| token.is(name))
        {
            return Err(error(
                token.offset,
                "Repository 禁止片段/外部 SQL 宏及 sqlx 编译期查询宏",
            ));
        }
        if token.is(".")
            && tokens.get(i + 1).is_some_and(|t| {
                [
                    "push_str",
                    "push",
                    "join",
                    "concat",
                    "extend",
                    "write_str",
                    "to_rfc3339",
                    "to_rfc3339_opts",
                ]
                .iter()
                .any(|name| t.is(name))
            })
        {
            return Err(error(
                token.offset,
                "Repository 禁止动态 SQL 拼接或自行编码时间",
            ));
        }
        if token.is("+")
            && (tokens
                .get(i.wrapping_sub(1))
                .is_some_and(|t| t.kind == Kind::Literal)
                || tokens
                    .get(i + 1)
                    .is_some_and(|t| t.kind == Kind::Literal || t.is("&")))
        {
            return Err(error(token.offset, "Repository 禁止字符串加法拼接 SQL"));
        }
        if token.is("format") && tokens.get(i + 1).is_some_and(|t| t.is("!")) {
            check_rust_format(&tokens, i)?;
        }
        if token.is(".") && tokens.get(i + 1).is_some_and(|t| t.is("replace")) {
            let pattern = tokens.get(i + 3).filter(|t| t.kind == Kind::Literal);
            if !pattern.is_some_and(|t| {
                t.text
                    .strip_prefix('{')
                    .and_then(|s| s.strip_suffix('}'))
                    .is_some_and(identifier)
            }) {
                return Err(error(
                    token.offset,
                    "Repository replace 仅允许完整标识符占位",
                ));
            }
            let end = closing_paren(&tokens, i + 2)?;
            if end <= i + 5 || !tokens[i + 4].is(",") {
                return Err(error(
                    token.offset,
                    "Repository 标识符替换必须有受信变量参数",
                ));
            }
            if tokens[i + 5..end].iter().any(|t| t.kind == Kind::Literal) {
                return Err(error(token.offset, "Repository 标识符替换必须使用受信变量"));
            }
        }
    }
    Ok(())
}

fn is_sql(text: &str) -> bool {
    let mut rest = text.trim_start();
    loop {
        if rest.starts_with("--") {
            rest = rest[rest.find('\n').unwrap_or(rest.len())..].trim_start();
        } else if rest.starts_with("/*") {
            let Ok(end) = block_comment_end(rest, 0, false) else {
                // 不能把非法/跨方言注释隐藏的 SQL 当作普通提示文本跳过。
                return true;
            };
            rest = rest[end..].trim_start();
        } else {
            break;
        }
    }
    let end = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphabetic() || *b == b'_')
        .count();
    [
        "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "REPLACE",
        "TRUNCATE", "PRAGMA",
    ]
    .iter()
    .any(|word| rest[..end].eq_ignore_ascii_case(word))
}

fn identifier(text: &str) -> bool {
    !text.is_empty()
        && !text.as_bytes()[0].is_ascii_digit()
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn check_rust_format(tokens: &[Token], i: usize) -> Result<(), LintError> {
    let Some(template) = tokens.get(i + 3).filter(|t| t.kind == Kind::Literal) else {
        return Err(error(tokens[i].offset, "format! 必须使用完整字面量模板"));
    };
    if !is_sql(&template.text) {
        // 非 SQL format! 允许，但拒绝用纯孔位格式和片段组装隐藏 SQL。
        let mut in_hole = false;
        let has_static_word = template.text.chars().any(|ch| match ch {
            '{' => {
                in_hole = true;
                false
            }
            '}' => {
                in_hole = false;
                false
            }
            _ => !in_hole && ch.is_alphabetic(),
        });
        if template.text.contains('{') && !has_static_word {
            return Err(error(
                template.offset,
                "Repository 禁止用纯占位 format! 组装 SQL",
            ));
        }
        return Ok(());
    }
    let end = closing_paren(tokens, i + 2)?;
    if tokens[i + 4..end].iter().any(|t| t.kind == Kind::Literal) {
        return Err(error(
            template.offset,
            "SQL format! 参数不能是 SQL/数据字面量片段",
        ));
    }
    Ok(())
}

fn rust_tokens(source: &str) -> Result<Vec<Token>, LintError> {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < source.len() {
        let rest = &source[i..];
        let ch = rest.chars().next().expect("非空");
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            i = block_comment_end(source, i, true)?;
            continue;
        }
        let offset = i;
        let raw_prefix = rest
            .strip_prefix('r')
            .map(|s| s.bytes().take_while(|b| *b == b'#').count());
        let raw_hashes = raw_prefix.filter(|hashes| rest.as_bytes().get(1 + hashes) == Some(&b'"'));
        let (text, kind) = if let Some(hashes) = raw_hashes {
            let start = i + hashes + 2;
            let closing = format!("\"{}", "#".repeat(hashes));
            let end = source[start..]
                .find(&closing)
                .ok_or_else(|| error(i, "Rust raw 字符串未闭合"))?
                + start;
            i = end + closing.len();
            (source[start..end].to_owned(), Kind::Literal)
        } else if ch == '"' {
            let (literal, end) = rust_string(source, i)?;
            i = end;
            (literal, Kind::Literal)
        } else if ch == '\''
            && let Some(end) = rust_char_end(source, i)
        {
            i = end;
            (String::new(), Kind::Literal)
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            i += rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            (source[offset..i].to_owned(), Kind::Word)
        } else {
            i += ch.len_utf8();
            (ch.to_string(), Kind::Symbol)
        };
        tokens.push(Token { text, offset, kind });
    }
    Ok(tokens)
}

fn rust_char_end(source: &str, start: usize) -> Option<usize> {
    let ch = source[start + 1..].chars().next()?;
    let mut end = start + 1 + ch.len_utf8();
    if ch == '\\' {
        let escaped = source[end..].chars().next()?;
        end += escaped.len_utf8();
        if escaped == 'u' {
            end += source[end..].find('}')? + 1;
        } else if escaped == 'x' {
            end += 2;
        }
    }
    (source.as_bytes().get(end) == Some(&b'\'')).then_some(end + 1)
}

fn rust_string(source: &str, start: usize) -> Result<(String, usize), LintError> {
    let mut result = String::new();
    let mut i = start + 1;
    loop {
        let ch = source[i..]
            .chars()
            .next()
            .ok_or_else(|| error(start, "Rust 字符串未闭合"))?;
        i += ch.len_utf8();
        match ch {
            '"' => return Ok((result, i)),
            '\\' => {
                let escaped = source[i..]
                    .chars()
                    .next()
                    .ok_or_else(|| error(i, "Rust 转义未闭合"))?;
                i += escaped.len_utf8();
                match escaped {
                    'n' => result.push('\n'),
                    'r' => result.push('\r'),
                    't' => result.push('\t'),
                    '0' => result.push('\0'),
                    '\\' | '\'' | '"' => result.push(escaped),
                    '\n' | '\r' => {
                        while source[i..].chars().next().is_some_and(char::is_whitespace) {
                            i += source[i..].chars().next().expect("非空").len_utf8();
                        }
                    }
                    'x' => {
                        let end = i + 2;
                        let text = source
                            .get(i..end)
                            .ok_or_else(|| error(i, "Rust 十六进制转义非法"))?;
                        let value = u8::from_str_radix(text, 16)
                            .map_err(|_| error(i, "Rust 十六进制转义非法"))?;
                        result.push(char::from(value));
                        i = end;
                    }
                    'u' => {
                        let rest = source[i..]
                            .strip_prefix('{')
                            .ok_or_else(|| error(i, "Rust Unicode 转义非法"))?;
                        let end = rest
                            .find('}')
                            .ok_or_else(|| error(i, "Rust Unicode 转义未闭合"))?;
                        let value = u32::from_str_radix(&rest[..end].replace('_', ""), 16)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or_else(|| error(i, "Rust Unicode 转义非法"))?;
                        result.push(value);
                        i += end + 2;
                    }
                    _ => return Err(error(i, "Rust 字符串转义非法")),
                }
            }
            _ => result.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn portable_queries_ctes_windows_and_subqueries_pass() {
        for sql in [
            "SELECT id FROM {table} WHERE id = ? LIMIT ?",
            "WITH batch AS (SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS n FROM {table}) SELECT id FROM batch WHERE n < ?",
            "UPDATE {table} SET value = (SELECT value FROM {other} WHERE id = ?) WHERE id = ?",
            "DELETE FROM {table} WHERE id IN (SELECT id FROM {other} WHERE value = ?)",
            "INSERT INTO {table} (id, value) VALUES (?, ?)",
            "SELECT id FROM {table} WHERE t.title_normalized NOT LIKE ? ESCAPE '!'",
            "UPDATE {table} SET value = (SELECT value FROM {other} WHERE id = ?) WHERE id NOT IN (?, ?)",
            "SELECT id FROM {table} WHERE title_normalized LIKE '%help%' ESCAPE '!'",
            "SELECT 'RETURNING CAST || \'\'quoted\'\'' FROM {table} /* CAST RETURNING :: */ -- FOR UPDATE\n WHERE id = ?",
        ] {
            lint_sql(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        }
    }

    #[test]
    fn all_forbidden_syntax_and_semantic_differences_fail() {
        for sql in [
            "INSERT INTO t VALUES (?) RETURNING id",
            "INSERT INTO t VALUES (?) ON CONFLICT DO NOTHING",
            "INSERT INTO t VALUES (?) ON DUPLICATE KEY UPDATE id = ?",
            "SELECT \"id\" FROM t",
            "SELECT `id` FROM t",
            "SELECT a || b FROM t",
            "UPDATE t SET a = s.a FROM s WHERE t.id = s.id",
            "WITH ids AS (SELECT id FROM s) UPDATE t JOIN ids ON t.id = ids.id SET a = ?",
            "UPDATE t, s SET t.a = ?",
            "DELETE t FROM t JOIN s ON t.id = s.id",
            "DELETE FROM t USING s WHERE t.id = s.id",
            "DELETE FROM t, s WHERE t.id = s.id",
            "SELECT id FROM t ORDER BY id NULLS FIRST",
            "SELECT id FROM t ORDER BY id NULLS LAST",
            "SELECT CAST(id AS TEXT) FROM t",
            "SELECT id FROM t WHERE a IS DISTINCT FROM b",
            "SELECT id FROM t WHERE a IS NOT DISTINCT FROM b",
            "DELETE FROM t WHERE id IN (SELECT id FROM s LIMIT ?)",
            "DELETE FROM t WHERE id NOT IN (SELECT id FROM s LIMIT ?)",
            "UPDATE t SET a = (SELECT a FROM s WHERE id = ?) FROM s WHERE t.id = s.id",
            "SELECT id FROM t WHERE id IN (WITH x AS (SELECT id FROM s LIMIT ?) SELECT id FROM x)",
            "SELECT id FROM t WHERE title LIKE ? ESCAPE '!'",
            "SELECT id FROM t WHERE title_normalized LIKE ?",
            "SELECT id FROM t WHERE title_normalized LIKE ? OR other_normalized LIKE ? ESCAPE '!'",
            "SELECT id FROM t WHERE title_normalized LIKE ? ESCAPE 'ab'",
            "SELECT id FROM t WHERE title_normalized ILIKE ?",
            "SELECT id FROM t FOR UPDATE",
            "SELECT id FROM t FOR UPDATE OF t",
            "SELECT id FROM t FOR NO KEY UPDATE",
            "SELECT id FROM t SKIP LOCKED",
            "SELECT id::text FROM t",
            "SELECT id FROM t WHERE id = $1",
            "SELECT id FROM t WHERE id = :id",
            "SELECT id FROM bcm_content.forums",
            "SELECT lastval()",
            "SELECT last_insert_rowid()",
            "SELECT LAST_INSERT_ID()",
            "SELECT id FROM {}",
            "SELECT id FROM {table:?}",
            "SELECT id FROM {table} {suffix}",
            "SELECT id FROM {table} WHERE id = {value}",
            "SELECT {columns} FROM {table}",
            "TRUNCATE TABLE t",
            "REPLACE INTO t VALUES (?)",
            "SELECT id FROM t /* outer /* inner */ RETURNING id -- */",
            "/* outer /* inner */ SELECT gen_random_uuid(); -- */",
            "SELECT E'plain' FROM t",
            r"SELECT E'\''; COMMIT; -- '",
            r"SELECT 'left\right' FROM t",
        ] {
            assert!(lint_sql(sql).is_err(), "应拒绝: {sql}");
        }
    }

    #[test]
    fn global_rules_cover_migrations_without_rejecting_acl() {
        for sql in [
            "CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;",
            "CREATE OR REPLACE PROCEDURE p() LANGUAGE SQL AS 'SELECT 1'",
            "CREATE FUNCTION p() RETURNS int AS 'SELECT 1' LANGUAGE SQL",
            "CREATE VIEW v AS SELECT id FROM t",
            "CREATE MATERIALIZED VIEW v AS SELECT id FROM t",
            "CREATE UNIQUE INDEX x ON t (id) WHERE active = 1",
            "CREATE TABLE t (id TEXT DEFAULT gen_random_uuid())",
            "INSERT INTO t VALUES (uuid_generate_v4())",
            "INSERT INTO t VALUES (randomblob(16))",
            "SELECT pg_catalog.gen_random_uuid()",
            "SELECT gen_random_bytes(16)",
            "SELECT RANDOM_BYTES(16)",
            "INSERT INTO t VALUES (UUID())",
            "INSERT INTO t VALUES (RAND())",
        ] {
            assert!(lint_migration_sql(sql).is_err(), "应拒绝: {sql}");
        }
        lint_migration_sql("CREATE ROLE app_rw NOINHERIT; REVOKE ALL ON SCHEMA {schema} FROM PUBLIC; ALTER DEFAULT PRIVILEGES FOR ROLE migrator IN SCHEMA {schema} REVOKE ALL ON TABLES FROM app_rw; GRANT SELECT ON {table} TO report_ro; CREATE INDEX idx ON {table} (id);").expect("ACL 和完整索引合法");
    }

    #[test]
    fn source_extracts_literals_and_ignores_comments_and_messages() {
        let valid = r####"
            // const BAD: &str = "SELECT CAST(id AS TEXT) FROM t";
            /* format!("SELECT id FROM t RETURNING id"); */
            /* Rust 注释允许嵌套 /* SELECT CAST(id AS TEXT) */ */
            const MESSAGE: &str = "禁止 RETURNING 和 CAST";
            const SQL: &str = r###"SELECT id FROM {table} WHERE id = ?"###;
            let sql = format!("SELECT id FROM {table} WHERE id = ?");
            let escaped = "SELECT '\"CAST\"' FROM {table}";
            fn borrow<'a>(text: &'a str) { let ch = '\''; }
        "####;
        lint_repo_source(valid).expect("注释和提示不是SQL");
        for source in [
            r#"const SQL: &str = "SELECT CAST(id AS TEXT) FROM t";"#,
            r##"let sql = r#"SELECT "id" FROM t"#;"##,
            r#"let sql = "SELECT \x43AST(id AS TEXT) FROM t";"#,
            r#"let sql = "SELECT \u{43}AST(id AS TEXT) FROM t";"#,
            r#"let sql = "SELECT 'unclosed";"#,
            r#"let sql = "/* outer /* inner */ SELECT gen_random_uuid(); -- */";"#,
            r#"let sql = "SELECT E'\\''; COMMIT; -- '";"#,
            r#"let sql = "SELECT id FROM t \
                RETURNING id";"#,
        ] {
            assert!(lint_repo_source(source).is_err(), "应拒绝SQL字面量");
        }
    }

    #[test]
    fn source_rejects_split_or_dynamic_sql_fail_closed() {
        for source in [
            r#"let sql = concat!("SEL", "ECT id FROM t RETURNING id");"#,
            r#"let sql = include_str!("query.sql");"#,
            r#"let sql = format!("{} {}", "SELECT", "id FROM t RETURNING id");"#,
            r#"let a = "SE"; let b = "LECT id FROM t RETURNING id"; let sql = format!("{a}{b}");"#,
            r#"let sql = format!("SELECT {tail}", tail = "id FROM t RETURNING id");"#,
            r#"let sql = "SEL".to_owned() + "ECT id FROM t RETURNING id";"#,
            r#"sql.push_str(" RETURNING id");"#,
            r#"let sql = fragments.join(" ");"#,
            r#"let sql = ["SE", "LECT id FROM t RETURNING id"].concat();"#,
            r#"let sql = "SELECT id FROM {table}".replace("{table}", "t RETURNING id");"#,
            r#"let sql = template.replace("SELECT", fragment);"#,
            r#"let result = sqlx::query!("SELECT id FROM t");"#,
            r#"let time = time.to_rfc3339();"#,
        ] {
            assert!(lint_repo_source(source).is_err(), "应拒绝拼接: {source}");
        }
    }

    #[test]
    fn malformed_sql_is_rejected() {
        for sql in [
            "SELECT 'oops",
            "SELECT (id FROM t",
            "SELECT id) FROM t",
            "/* open",
        ] {
            assert!(lint_sql(sql).is_err());
        }
    }

    fn collect(path: &Path, in_repo: bool, files: &mut Vec<PathBuf>) {
        let mut entries = std::fs::read_dir(path)
            .expect("扫描工作区目录")
            .map(|entry| entry.expect("读取目录项").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let name = path.file_name().expect("文件名").to_string_lossy();
            if path.is_dir() {
                if !matches!(name.as_ref(), "target" | "node_modules" | ".git" | "static") {
                    collect(&path, in_repo || name == "repo", files);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && (in_repo || name == "repo.rs" || name.ends_with("_repo.rs"))
            {
                files.push(path);
            }
        }
    }

    #[test]
    fn workspace_repository_sql_obeys_portable_subset() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        collect(&root.join("crates"), false, &mut files);
        collect(&root.join("apps"), false, &mut files);
        assert!(!files.is_empty(), "必须实际扫描 Repository 文件");
        for file in files {
            let source = std::fs::read_to_string(&file).expect("读取UTF-8 Repository");
            lint_repo_source(&source).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        }
    }

    #[test]
    fn workspace_migrations_obey_global_bans() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
        for backend in ["postgres", "sqlite", "mysql8"] {
            for entry in std::fs::read_dir(root.join(backend)).expect("读取迁移轨") {
                let file = entry.expect("迁移文件").path();
                if file.extension().is_some_and(|ext| ext == "sql") {
                    let sql = std::fs::read_to_string(&file).expect("读取UTF-8迁移");
                    lint_migration_sql(&sql)
                        .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
                }
            }
        }
    }
}
