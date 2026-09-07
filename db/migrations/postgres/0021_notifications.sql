-- WP-MVP-C04 切片：NOTIFY-001／002／003 站内通知。
--
-- 设计要点：
--   * `idempotency_key` UNIQUE 是 NOTIFY-003 的落点：任务重试与并发投递都靠它
--     兜底，而不是投递前先查一次。先查再插在并发下必然漏——两路同时查到
--     「还没有」就各插一条，用户会看到两条一模一样的通知。
--   * `read_at` 用哨兵时间戳表示未读，与 community 侧 `deleted_at` 的既有约定
--     一致：可空列会让「未读」在 SQL 里变成 NULL 语义，而三库对 NULL 的排序与
--     比较差异正是 DEC-022 要避开的。
--   * kind 用 CHECK 封闭枚举。`adopted` 按 NOTIFY-001 只预留类型，MVP 不产生
--     该类通知——预留在库里而不是等到 P1-A 再加迁移，是为了让类型集合从一开始
--     就是完整的，避免将来为一个枚举值再动一次表。
--   * 不存未读计数。计数由聚合得出，冗余计数与已读状态一旦不一致，用户会看到
--     「有未读但点进去是空的」，那种 bug 极难复现。
CREATE TABLE {{table:interaction.notifications}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    recipient_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (recipient_id > 0),
    kind TEXT NOT NULL CHECK (kind IN ('reply', 'mention', 'quote', 'favorite_update', 'adopted', 'moderation', 'account_security')),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('topic', 'post', 'case', 'account')),
    source_id BIGINT NOT NULL CHECK (source_id > 0),
    -- 触发者。系统事件（审核、账号安全）没有触发者，存 0；不用可空列，理由同上。
    actor_id BIGINT NOT NULL CHECK (actor_id >= 0),
    idempotency_key TEXT NOT NULL UNIQUE,
    read_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE INDEX {{index:interaction.notification_inbox_idx}} ON {{table:interaction.notifications}} (recipient_id, read_at, id);

REVOKE ALL ON TABLE {{table:interaction.notifications}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:interaction.notifications}} TO {{role:app_rw}};
-- Worker 需要投递通知（Outbox 消费方），因此同样授写。
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:interaction.notifications}} TO {{role:worker_rw}};
GRANT USAGE ON SCHEMA {{schema:interaction}} TO {{role:worker_rw}};
REVOKE ALL ON SEQUENCE {{sequence:interaction.notifications.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:interaction.notifications.id}} TO {{role:app_rw}}, {{role:worker_rw}};
