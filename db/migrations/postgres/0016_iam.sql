-- WP-MVP-D04 波次一：IAM catalogue、角色分配、ABAC 范围与高危审批（DEC-058）。
--
-- 设计要点：
--   * roles / role_permissions 是 catalogue，由 crates/manage/iam/registry.toml 生成，
--     运行时对应用角色**只读**（03 §4）：授权状态变更走受控命令，不由 Web 直接改目录。
--   * role_assignments 带开始、到期、撤销与版本：到期与撤销都靠时间比较自然生效，
--     不依赖清理任务，否则一个没跑的定时任务会让过期权限继续有效。
--   * assignment_scopes 是 ABAC 范围（03 §5）：global 不带资源 ID，forums 带版块 ID 集合。
--     数字 ID 必须同时携带资源类型，防止同值 ID 跨域复用。
--   * authz_version 记在 users 上：权限变更后递增，使旧管理会话立即失效。
--   * approvals 记录高危操作的双人审批；申请人与审批人必须不同，服务端校验。
--
-- 本迁移不创建任何账号，也不给任何账号分配角色——首个所有者仍只能走 AUTH-014。
-- ACL：catalogue 只授 SELECT，分配与审批授必要写权限；worker_rw/report_ro/PUBLIC 一律 REVOKE。
ALTER TABLE {{table:identity.users}} ADD COLUMN authz_version BIGINT NOT NULL DEFAULT 1 CHECK (authz_version > 0);
CREATE TABLE {{table:iam.roles}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    role_key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:iam.role_permissions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    role_key TEXT NOT NULL,
    permission_key TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    UNIQUE (role_key, permission_key)
);
CREATE TABLE {{table:iam.role_assignments}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    role_key TEXT NOT NULL,
    granted_by BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    starts_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:iam.assignment_owner_idx}} ON {{table:iam.role_assignments}} (user_id, revoked_at);
CREATE TABLE {{table:iam.assignment_scopes}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    assignment_id BIGINT NOT NULL REFERENCES {{table:iam.role_assignments}} (id),
    scope_type TEXT NOT NULL CHECK (scope_type IN ('global', 'forums')),
    resource_type TEXT NOT NULL,
    resource_id BIGINT NOT NULL CHECK (resource_id >= 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:iam.scope_assignment_idx}} ON {{table:iam.assignment_scopes}} (assignment_id);
CREATE TABLE {{table:iam.approvals}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    permission_key TEXT NOT NULL,
    requester_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    operator_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    approver_id BIGINT NOT NULL CHECK (approver_id >= 0),
    target_kind TEXT NOT NULL,
    target_id BIGINT NOT NULL CHECK (target_id >= 0),
    state TEXT NOT NULL CHECK (state IN ('pending', 'approved', 'revoked')),
    created_at TIMESTAMPTZ NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:iam.approval_lookup_idx}} ON {{table:iam.approvals}} (operator_id, permission_key, state);
REVOKE ALL ON TABLE {{table:iam.roles}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:iam.roles}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:iam.roles.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON TABLE {{table:iam.role_permissions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:iam.role_permissions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:iam.role_permissions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON TABLE {{table:iam.role_assignments}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:iam.role_assignments}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:iam.role_assignments.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:iam.role_assignments.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:iam.assignment_scopes}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:iam.assignment_scopes}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:iam.assignment_scopes.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:iam.assignment_scopes.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:iam.approvals}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:iam.approvals}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:iam.approvals.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:iam.approvals.id}} TO {{role:app_rw}};
