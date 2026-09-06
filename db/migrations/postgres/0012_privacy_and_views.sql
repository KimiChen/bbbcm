-- DEC-054 的逐用户隐私设置与浏览量统计。
--
-- identity.users 增加两列，默认真，对应 DEC-054 批准的「作者列表与正文默认公开」：
--   * profile_lists_public 对应 PRIV-001，个人主页的主题/回帖列表是否对未登录访客可见；
--   * topic_body_public   对应 PRIV-002，本人主题正文是否对未登录访客可见（标题另按规则保留）。
-- 默认值只是账号级默认，用户可在 /settings/privacy 逐项关闭。
--
-- community.topic_views 是浏览去重台账，不是浏览明细：
--   * subject_hash 为用途隔离 HMAC 派生的不可逆摘要，原始用户 ID / Cookie / 设备指纹一律不落库；
--   * (topic_id, subject_hash) 唯一，配合 last_viewed_at 实现 DEC-054 的 30 分钟去重窗口；
--   * 窗口内重复访问只更新 last_viewed_at，不增加 topics.view_count。
-- topics.view_count 是累计阅读量，同时供 NAV-003 主题行展示与 NAV-004 热门排序（DEC-030）使用。
--
-- ACL：只授运行角色 app_rw 必要权限，worker_rw / report_ro / PUBLIC 一律 REVOKE。
ALTER TABLE {{table:identity.users}} ADD COLUMN profile_lists_public BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE {{table:identity.users}} ADD COLUMN topic_body_public BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE {{table:community.topics}} ADD COLUMN view_count BIGINT NOT NULL DEFAULT 0 CHECK (view_count >= 0);
CREATE TABLE {{table:community.topic_views}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    topic_id BIGINT NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    subject_hash TEXT NOT NULL,
    first_viewed_at TIMESTAMPTZ NOT NULL,
    last_viewed_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    UNIQUE (topic_id, subject_hash)
);
CREATE INDEX {{index:community.topic_view_subject_idx}} ON {{table:community.topic_views}} (subject_hash);
REVOKE ALL ON TABLE {{table:community.topic_views}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:community.topic_views}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:community.topic_views.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:community.topic_views.id}} TO {{role:app_rw}};
