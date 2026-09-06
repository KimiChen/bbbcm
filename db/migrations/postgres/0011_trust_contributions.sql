-- DEC-054 有效贡献事实表，供 trust 的确定性 T1 授级规则统计。
--
-- 设计要点：
--   * 每条合格的已发布内容对应一行，post_id 唯一，保证命令重放不会重复计数。
--   * counted 为标志位而非删除行：本人删除只置为否，事实行保留，
--     使贡献总数随时可以从追加历史重算（TRUST-002 要求可解释、可重算）。
--   * body_chars 记录发布时的 Unicode 标量长度（DEC-055），便于事后复核判定依据。
--   * rule_version 记录评估时的规则版本，规则变更后可区分历史数据。
--
-- 本迁移不授予任何信任等级、不写入发布策略、不新增账号或 Feature Flag。
-- ACL：只授运行角色 app_rw 必要权限，worker_rw / report_ro / PUBLIC 一律 REVOKE。
CREATE TABLE {{table:trust.contribution_facts}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    kind TEXT NOT NULL CHECK (kind IN ('topic', 'reply')),
    post_id BIGINT NOT NULL UNIQUE CHECK (post_id > 0),
    body_chars BIGINT NOT NULL CHECK (body_chars >= 0),
    counted BOOLEAN NOT NULL,
    rule_version BIGINT NOT NULL CHECK (rule_version > 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:trust.contribution_owner_idx}} ON {{table:trust.contribution_facts}} (user_id, counted);
REVOKE ALL ON TABLE {{table:trust.contribution_facts}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:trust.contribution_facts}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:trust.contribution_facts.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:trust.contribution_facts.id}} TO {{role:app_rw}};
