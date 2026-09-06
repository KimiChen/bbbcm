-- DEC-054 的逐用户隐私设置与浏览量统计，与 postgres 轨同编号配对，字段语义完全一致。
--
-- profile_lists_public 对应 PRIV-001，topic_body_public 对应 PRIV-002，默认真即默认公开。
-- topic_views 是去重台账而非浏览明细：subject_hash 为不可逆摘要，(topic_id, subject_hash)
-- 唯一，配合 last_viewed_at 实现 30 分钟去重窗口。SQLite 无角色体系，故不含 ACL 语句。
ALTER TABLE {{table:identity.users}} ADD COLUMN profile_lists_public INTEGER NOT NULL DEFAULT 1 CHECK (profile_lists_public IN (0, 1));
ALTER TABLE {{table:identity.users}} ADD COLUMN topic_body_public INTEGER NOT NULL DEFAULT 1 CHECK (topic_body_public IN (0, 1));
ALTER TABLE {{table:community.topics}} ADD COLUMN view_count BIGINT NOT NULL DEFAULT 0 CHECK (view_count >= 0);
CREATE TABLE {{table:community.topic_views}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    topic_id BIGINT NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    subject_hash TEXT NOT NULL,
    first_viewed_at TEXT NOT NULL,
    last_viewed_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    UNIQUE (topic_id, subject_hash)
);
CREATE INDEX {{index:community.topic_view_subject_idx}} ON {{table:community.topic_views}} (subject_hash);
