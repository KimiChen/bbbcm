-- rebuild
-- Content risk kinds and read-only active restriction authority. No grants or policy seed.
CREATE TABLE {{rebuild:trust.risk_events}} (
    event_version BIGINT NOT NULL CHECK (event_version = 1),
    event_id TEXT PRIMARY KEY NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('registration', 'login', 'invitation', 'topic', 'reply', 'edit', 'delete')),
    actor_id BIGINT NOT NULL CHECK (actor_id >= 0),
    occurred_at TEXT NOT NULL,
    rule_version BIGINT NOT NULL CHECK (rule_version >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'challenge', 'unavailable')),
    reason_code TEXT NOT NULL,
    signals_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL
);
INSERT INTO {{rebuild:trust.risk_events}} SELECT * FROM {{table:trust.risk_events}};
DROP TABLE {{table:trust.risk_events}};
ALTER TABLE {{rebuild:trust.risk_events}} RENAME TO {{table:trust.risk_events}};
CREATE INDEX {{index:trust.risk_actor_idx}} ON {{table:trust.risk_events}} (actor_id, occurred_at);
CREATE TABLE {{table:moderation.content_restrictions}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    capability TEXT NOT NULL CHECK (capability IN ('content', 'topic', 'reply', 'edit', 'delete')),
    starts_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked INTEGER NOT NULL CHECK (revoked IN (0, 1)),
    version BIGINT NOT NULL CHECK (version > 0),
    CHECK (expires_at > starts_at)
);
CREATE INDEX {{index:moderation.content_restriction_owner_idx}} ON {{table:moderation.content_restrictions}} (user_id, revoked, starts_at, expires_at);
