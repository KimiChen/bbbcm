-- WP-MVP-B03：staff principal、TOTP、恢复码、管理会话与 MFA assertion。
--
-- 设计要点：
--   * staff_principals 把「是不是 staff」与普通账号解耦。普通会话永远不能升级成管理
--     会话，两者是不同的凭据体系（03 §6：独立 cookie 与路径）。
--   * mfa_credentials 按 DEC-062 只存 AEAD 密文、nonce 与密钥版本，绝不存明文共享密钥；
--     last_step 记录最近一次验签成功的时间步，用于拒绝同一时间步内的重放。
--   * mfa_recovery_codes 只存哈希、单次消费（03 §6），consumed_at 为空表示尚未使用。
--   * management_sessions 与普通会话分表：绝对上限与空闲上限分列，便于按 03 §6 分别校验。
--   * mfa_assertions 是高危操作的重新认证凭据：单次消费，且同时绑定用户、管理会话与用途，
--     防止把「改密码时做的验证」拿去当「授予特权角色」的凭据。
--
-- 本迁移不授予任何角色、不创建任何账号、不写入权限种子。
-- ACL：只授运行角色 app_rw 必要权限，worker_rw / report_ro / PUBLIC 一律 REVOKE。
CREATE TABLE {{table:identity.staff_principals}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.users}} (id),
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:identity.mfa_credentials}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.users}} (id),
    secret_cipher TEXT NOT NULL,
    secret_nonce TEXT NOT NULL,
    key_version BIGINT NOT NULL CHECK (key_version > 0),
    confirmed_at TIMESTAMPTZ NOT NULL,
    last_step BIGINT NOT NULL CHECK (last_step >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:identity.mfa_recovery_codes}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    code_hash TEXT NOT NULL UNIQUE,
    consumed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:identity.recovery_owner_idx}} ON {{table:identity.mfa_recovery_codes}} (user_id, consumed_at);
CREATE TABLE {{table:identity.management_sessions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    session_hash TEXT NOT NULL UNIQUE,
    csrf_hash TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    created_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    absolute_expires_at TIMESTAMPTZ NOT NULL,
    mfa_bound BOOLEAN NOT NULL,
    revoked BOOLEAN NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    CHECK (absolute_expires_at > created_at)
);
CREATE INDEX {{index:identity.management_owner_idx}} ON {{table:identity.management_sessions}} (user_id, revoked);
CREATE TABLE {{table:identity.mfa_assertions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    assertion_hash TEXT NOT NULL UNIQUE,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    management_session_id BIGINT NOT NULL REFERENCES {{table:identity.management_sessions}} (id),
    purpose TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
REVOKE ALL ON TABLE {{table:identity.staff_principals}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.staff_principals}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.staff_principals.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.staff_principals.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.mfa_credentials}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.mfa_credentials}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.mfa_credentials.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.mfa_credentials.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.mfa_recovery_codes}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.mfa_recovery_codes}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.mfa_recovery_codes.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.mfa_recovery_codes.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.management_sessions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.management_sessions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.management_sessions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.management_sessions.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.mfa_assertions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.mfa_assertions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.mfa_assertions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.mfa_assertions.id}} TO {{role:app_rw}};
