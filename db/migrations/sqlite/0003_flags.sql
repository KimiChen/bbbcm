-- A02 read-only slice. Migration initializes registered defaults once; startup never repairs.
CREATE TABLE {{table:platform.feature_flags}} (
    flag_key TEXT PRIMARY KEY NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    version BIGINT NOT NULL CHECK (version > 0)
);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('public_read', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('registration', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('email_verification', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('topic_creation', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('reply_creation', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('attachments', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('search', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('rss', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('reporting', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('support', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('manage_console', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('registration_lock', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('invitation_lock', 0, 1);
INSERT INTO {{table:platform.feature_flags}} (flag_key, enabled, version) VALUES ('read_only_mode', 0, 1);
