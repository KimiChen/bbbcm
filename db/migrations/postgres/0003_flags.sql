-- A02 read-only slice. Migration initializes registered defaults once; startup never repairs.
CREATE TABLE {{table:platform.feature_flags}} (
    flag_key TEXT PRIMARY KEY NOT NULL,
    enabled BOOLEAN NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('public_read', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('registration', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('email_verification', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('topic_creation', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('reply_creation', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('attachments', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('search', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('rss', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('reporting', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('support', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('manage_console', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('registration_lock', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('invitation_lock', FALSE, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('read_only_mode', FALSE, 1);
REVOKE ALL ON TABLE {{table:platform.feature_flags}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:platform.feature_flags}} TO {{role:app_rw}}, {{role:worker_rw}};
