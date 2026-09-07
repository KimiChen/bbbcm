-- WP-MVP-C04 切片：NOTIFY-001／002／003 站内通知。与 postgres 轨同编号配对，
-- 字段语义完全一致；SQLite 无角色体系，故不含 ACL 语句。
--
-- idempotency_key 的 UNIQUE 与 kind 的 CHECK 在两个后端上都成立：
-- NOTIFY-003 的幂等保证不能只在完整模式生效，否则体验模式会重复投递。
CREATE TABLE {{table:interaction.notifications}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recipient_id INTEGER NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (recipient_id > 0),
    kind TEXT NOT NULL CHECK (kind IN ('reply', 'mention', 'quote', 'favorite_update', 'adopted', 'moderation', 'account_security')),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('topic', 'post', 'case', 'account')),
    source_id INTEGER NOT NULL CHECK (source_id > 0),
    actor_id INTEGER NOT NULL CHECK (actor_id >= 0),
    idempotency_key TEXT NOT NULL UNIQUE,
    read_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:interaction.notification_inbox_idx}} ON {{table:interaction.notifications}} (recipient_id, read_at, id);
