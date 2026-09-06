-- WP-MVP-D02：处置记录与申诉。
-- 与 postgres 轨同编号配对，字段语义完全一致；SQLite 无角色体系，故不含 ACL 语句。
--
-- 设计要点：
--   * enforcement_actions 是处置的唯一事实源。内容状态变更与账号限制都由处置命令驱动，
--     不再有第二条修改路径，否则「处罚历史」与「当前状态」迟早对不上。
--   * 处罚历史只追加：撤销写 revoked_at 与撤销人，不删除原记录（MOD-006）。
--   * expires_at 为纪元表示永久处罚；临时处罚到期后限制自然失效，不需要清理任务。
--   * before_state / after_state 记录动作前后的内容或账号状态，供事后复核与申诉举证。
--   * appeals 与处置一对一：MOD-011 只给一次正常申诉，唯一键直接把「重复申诉」挡在库层。
--     reviewer_id 与处置执行人不同由服务端校验，库层只保证一次性。
--
-- 本迁移不创建账号、不授予角色、不写入权限种子。
-- ACL：只授运行角色 app_rw 必要权限，worker_rw / report_ro / PUBLIC 一律 REVOKE。
CREATE TABLE {{table:moderation.enforcement_actions}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id BIGINT NOT NULL CHECK (case_id >= 0),
    action TEXT NOT NULL CHECK (action IN ('ignore', 'obscure', 'hide', 'soft_delete', 'lock', 'move', 'warn', 'mute', 'suspend', 'ban')),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('topic', 'post', 'account')),
    target_id BIGINT NOT NULL CHECK (target_id > 0),
    subject_id BIGINT NOT NULL CHECK (subject_id > 0),
    operator_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    reason_code TEXT NOT NULL,
    rule_version BIGINT NOT NULL CHECK (rule_version > 0),
    evidence_digest TEXT NOT NULL,
    before_state TEXT NOT NULL,
    after_state TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT NOT NULL,
    revoked_by BIGINT NOT NULL CHECK (revoked_by >= 0),
    revoke_reason TEXT NOT NULL,
    created_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:moderation.enforcement_subject_idx}} ON {{table:moderation.enforcement_actions}} (subject_id, revoked_at, created_at);
CREATE INDEX {{index:moderation.enforcement_case_idx}} ON {{table:moderation.enforcement_actions}} (case_id, id);
CREATE TABLE {{table:moderation.appeals}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id BIGINT NOT NULL UNIQUE REFERENCES {{table:moderation.enforcement_actions}} (id),
    appellant_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    statement TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('pending', 'upheld', 'overturned', 'reduced')),
    reviewer_id BIGINT NOT NULL CHECK (reviewer_id >= 0),
    review_note TEXT NOT NULL,
    created_at TEXT NOT NULL,
    reviewed_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
