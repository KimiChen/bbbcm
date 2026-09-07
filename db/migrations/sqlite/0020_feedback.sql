-- WP-MVP-C04 切片：SOCIAL-002／SOCIAL-003 正向反馈。与 postgres 轨同编号配对，
-- 字段语义完全一致；SQLite 无角色体系，故不含 ACL 语句。
--
-- 唯一性同样由数据库约束兜底（UNIQUE），自我反馈同样由 CHECK 拒绝——
-- 两条约束在两个后端上必须都成立，否则体验模式会成为绕过口。
CREATE TABLE {{table:interaction.feedback}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (user_id > 0),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('topic', 'post')),
    target_id INTEGER NOT NULL CHECK (target_id > 0),
    author_id INTEGER NOT NULL CHECK (author_id > 0),
    withdrawn INTEGER NOT NULL CHECK (withdrawn IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    CONSTRAINT feedback_once UNIQUE (user_id, target_kind, target_id),
    CONSTRAINT feedback_not_self CHECK (user_id <> author_id)
);
CREATE INDEX {{index:interaction.feedback_target_idx}} ON {{table:interaction.feedback}} (target_kind, target_id, withdrawn);
