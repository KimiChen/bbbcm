-- A06 mail side-effect metadata only. Message content remains in its trusted source.
CREATE TABLE {{table:platform.mail_dispatches}} (
    message_id TEXT PRIMARY KEY NOT NULL,
    event_id TEXT UNIQUE NOT NULL REFERENCES {{table:platform.outbox_events}} (event_id),
    recipient_ref TEXT NOT NULL,
    class TEXT NOT NULL CHECK (class IN ('security', 'transactional', 'marketing')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'local_recorded', 'accepted', 'suppressed', 'failed')),
    request_hash TEXT NOT NULL,
    simulated INTEGER NOT NULL CHECK (simulated IN (0, 1)),
    queued_at TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:platform.mail_suppressions}} (
    recipient_ref TEXT PRIMARY KEY NOT NULL,
    unsubscribe_marketing INTEGER NOT NULL CHECK (unsubscribe_marketing IN (0, 1)),
    complained INTEGER NOT NULL CHECK (complained IN (0, 1)),
    bounced INTEGER NOT NULL CHECK (bounced IN (0, 1)),
    hard_bounces INTEGER NOT NULL CHECK (hard_bounces >= 0),
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:platform.mail_feedback}} (
    provider TEXT NOT NULL,
    feedback_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES {{table:platform.mail_dispatches}} (message_id),
    kind TEXT NOT NULL CHECK (kind IN ('delivered', 'delayed', 'soft_bounce', 'hard_bounce', 'complaint', 'unsubscribe')),
    fingerprint TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    received_at TEXT NOT NULL,
    simulated INTEGER NOT NULL CHECK (simulated IN (0, 1)),
    PRIMARY KEY (provider, feedback_id)
);
CREATE TABLE {{table:platform.mail_alerts}} (
    alert_id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    feedback_id TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('hard_bounce', 'complaint')),
    created_at TEXT NOT NULL,
    UNIQUE (provider, feedback_id),
    FOREIGN KEY (provider, feedback_id) REFERENCES {{table:platform.mail_feedback}} (provider, feedback_id)
);
CREATE INDEX {{index:platform.mail_recipient_idx}} ON {{table:platform.mail_dispatches}} (recipient_ref, message_id);
CREATE INDEX {{index:platform.mail_observed_idx}} ON {{table:platform.mail_dispatches}} (simulated, queued_at, state);
CREATE INDEX {{index:platform.mail_feedback_time_idx}} ON {{table:platform.mail_feedback}} (simulated, occurred_at, kind);
