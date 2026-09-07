-- WP-MVP-C04 切片：SOCIAL-002／SOCIAL-003 正向反馈。
--
-- 设计要点：
--   * 唯一性由**数据库约束**兜底，不靠应用层「先查再写」。SOCIAL-003 要求
--     同一用户对同一对象只能产生一次有效反馈，而先查再写在并发下必然漏——
--     两路同时查到「还没有」就各写一条。UNIQUE 让第二路直接失败。
--   * 撤销不删行，只翻 withdrawn 标记。删行会让「撤销后重新反馈」变成又一条
--     新行，唯一约束反而挡不住；保留行则由同一条记录承载状态变化，也留下痕迹。
--   * target_kind 用 CHECK 封闭枚举，不用自由文本：类型要参与计数口径与后续
--     的「值得看」区域阈值，自由文本会让两者失控。
--   * 不在此存计数。计数由读路径聚合或后续投影维护，冗余计数列与唯一约束
--     一旦不一致，排查成本远高于一次聚合查询。
CREATE TABLE {{table:interaction.feedback}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (user_id > 0),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('topic', 'post')),
    target_id BIGINT NOT NULL CHECK (target_id > 0),
    -- 被反馈内容的作者。SOCIAL-003 禁止自我反馈，把作者存进来是为了让约束
    -- 可以在库内表达，而不是每次都回查内容表。
    author_id BIGINT NOT NULL CHECK (author_id > 0),
    withdrawn BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    CONSTRAINT feedback_once UNIQUE (user_id, target_kind, target_id),
    -- 自我反馈在库层直接拒绝：应用层校验会被新代码路径绕过，CHECK 不会。
    CONSTRAINT feedback_not_self CHECK (user_id <> author_id)
);
CREATE INDEX {{index:interaction.feedback_target_idx}} ON {{table:interaction.feedback}} (target_kind, target_id, withdrawn);

GRANT USAGE ON SCHEMA {{schema:interaction}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:interaction.feedback}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:interaction.feedback}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:interaction.feedback.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:interaction.feedback.id}} TO {{role:app_rw}};
