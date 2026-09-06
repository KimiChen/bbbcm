-- DEC-054 有效贡献事实表，供 trust 的确定性 T1 授级规则统计。
--
-- 与 postgres 轨同编号配对，字段语义完全一致：post_id 唯一保证幂等，
-- counted 为标志位以保留可重算的追加历史，body_chars 为 Unicode 标量长度（DEC-055），
-- rule_version 记录评估时的规则版本。SQLite 无角色体系，故不含 ACL 语句。
CREATE TABLE {{table:trust.contribution_facts}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    kind TEXT NOT NULL CHECK (kind IN ('topic', 'reply')),
    post_id BIGINT NOT NULL UNIQUE CHECK (post_id > 0),
    body_chars BIGINT NOT NULL CHECK (body_chars >= 0),
    counted INTEGER NOT NULL CHECK (counted IN (0, 1)),
    rule_version BIGINT NOT NULL CHECK (rule_version > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:trust.contribution_owner_idx}} ON {{table:trust.contribution_facts}} (user_id, counted);
