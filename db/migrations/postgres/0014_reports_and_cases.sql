-- WP-MVP-D01：举报、案件、内部备注与证据快照。
--
-- 设计要点：
--   * 每个目标至多一个未关闭案件（closed_at 为纪元表示未关闭），新举报挂到既有案件，
--     因此「同一条内容被 50 人举报」是一个案件 50 条举报，而不是 50 个案件。
--   * reports 的 (reporter_id, case_id) 唯一：同一人对同一案件只登记一次举报，
--     重复提交返回既有工单，避免刷举报把队列淹掉（MOD-001 去重）。
--   * evidence_snapshots 保存目标内容在举报时刻的不可变副本与 SHA-256 摘要（MOD-008）。
--     正文属 C2 受限，只能由授权管理路径读取，普通读路径永不返回。
--   * case_notes 是内部备注，同样 C2；与用户可见的处罚理由（MOD-007，属 D02）分开存放。
--   * escalated 标记 MOD-012 的六类高危请求，本轮只保证「标记不会丢」。
--
-- 本迁移不创建任何账号、不授予任何角色、不写入权限种子。
-- ACL：只授运行角色 app_rw 必要权限，worker_rw / report_ro / PUBLIC 一律 REVOKE。
CREATE TABLE {{table:moderation.cases}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('topic', 'post', 'profile')),
    target_id BIGINT NOT NULL CHECK (target_id > 0),
    status TEXT NOT NULL CHECK (status IN ('triage', 'investigating', 'awaiting_info', 'resolved', 'appealed', 'closed')),
    priority BIGINT NOT NULL CHECK (priority BETWEEN 1 AND 3),
    escalated BOOLEAN NOT NULL,
    assignee_id BIGINT NOT NULL CHECK (assignee_id >= 0),
    report_count BIGINT NOT NULL CHECK (report_count >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:moderation.case_target_idx}} ON {{table:moderation.cases}} (target_kind, target_id, closed_at);
CREATE INDEX {{index:moderation.case_queue_idx}} ON {{table:moderation.cases}} (status, priority, id);
CREATE TABLE {{table:moderation.reports}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    case_id BIGINT NOT NULL REFERENCES {{table:moderation.cases}} (id),
    reporter_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    category TEXT NOT NULL CHECK (category IN ('spam', 'harassment', 'illegal', 'privacy', 'infringement', 'fraud', 'malware', 'other')),
    note TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    UNIQUE (case_id, reporter_id)
);
CREATE TABLE {{table:moderation.case_notes}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    case_id BIGINT NOT NULL REFERENCES {{table:moderation.cases}} (id),
    author_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:moderation.case_note_idx}} ON {{table:moderation.case_notes}} (case_id, id);
CREATE TABLE {{table:moderation.evidence_snapshots}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    case_id BIGINT NOT NULL REFERENCES {{table:moderation.cases}} (id),
    captured_body TEXT NOT NULL,
    body_digest TEXT NOT NULL,
    captured_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:moderation.evidence_case_idx}} ON {{table:moderation.evidence_snapshots}} (case_id, id);
REVOKE ALL ON TABLE {{table:moderation.cases}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:moderation.cases}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.cases.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:moderation.cases.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:moderation.reports}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:moderation.reports}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.reports.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:moderation.reports.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:moderation.case_notes}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:moderation.case_notes}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.case_notes.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:moderation.case_notes.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:moderation.evidence_snapshots}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:moderation.evidence_snapshots}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.evidence_snapshots.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:moderation.evidence_snapshots.id}} TO {{role:app_rw}};
