-- A06 mail side-effect metadata only. Message content remains in its trusted source.
GRANT USAGE ON SCHEMA {{schema:platform}} TO {{role:report_ro}};
CREATE TABLE {{table:platform.mail_dispatches}} (
    message_id TEXT PRIMARY KEY NOT NULL,
    event_id TEXT UNIQUE NOT NULL REFERENCES {{table:platform.outbox_events}} (event_id),
    recipient_ref TEXT NOT NULL,
    class TEXT NOT NULL CHECK (class IN ('security', 'transactional', 'marketing')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'local_recorded', 'accepted', 'suppressed', 'failed')),
    request_hash TEXT NOT NULL,
    simulated BOOLEAN NOT NULL,
    queued_at TIMESTAMPTZ NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:platform.mail_suppressions}} (
    recipient_ref TEXT PRIMARY KEY NOT NULL,
    unsubscribe_marketing BOOLEAN NOT NULL,
    complained BOOLEAN NOT NULL,
    bounced BOOLEAN NOT NULL,
    hard_bounces BIGINT NOT NULL CHECK (hard_bounces >= 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:platform.mail_feedback}} (
    provider TEXT NOT NULL,
    feedback_id TEXT NOT NULL,
    message_id TEXT NOT NULL REFERENCES {{table:platform.mail_dispatches}} (message_id),
    kind TEXT NOT NULL CHECK (kind IN ('delivered', 'delayed', 'soft_bounce', 'hard_bounce', 'complaint', 'unsubscribe')),
    fingerprint TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL,
    simulated BOOLEAN NOT NULL,
    PRIMARY KEY (provider, feedback_id)
);
CREATE TABLE {{table:platform.mail_alerts}} (
    alert_id TEXT PRIMARY KEY NOT NULL,
    provider TEXT NOT NULL,
    feedback_id TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('hard_bounce', 'complaint')),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (provider, feedback_id),
    FOREIGN KEY (provider, feedback_id) REFERENCES {{table:platform.mail_feedback}} (provider, feedback_id)
);
CREATE INDEX {{index:platform.mail_recipient_idx}} ON {{table:platform.mail_dispatches}} (recipient_ref, message_id);
CREATE INDEX {{index:platform.mail_observed_idx}} ON {{table:platform.mail_dispatches}} (simulated, queued_at, state);
CREATE INDEX {{index:platform.mail_feedback_time_idx}} ON {{table:platform.mail_feedback}} (simulated, occurred_at, kind);
REVOKE ALL ON TABLE {{table:platform.mail_dispatches}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:platform.mail_dispatches}} TO {{role:worker_rw}};
REVOKE ALL ON TABLE {{table:platform.mail_suppressions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:platform.mail_suppressions}} TO {{role:worker_rw}};
REVOKE ALL ON TABLE {{table:platform.mail_feedback}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:platform.mail_feedback}} TO {{role:worker_rw}};
REVOKE ALL ON TABLE {{table:platform.mail_alerts}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:platform.mail_alerts}} TO {{role:worker_rw}};
GRANT SELECT (class, state, simulated, queued_at, observed_at) ON TABLE {{table:platform.mail_dispatches}} TO {{role:report_ro}};
GRANT SELECT (kind, simulated, occurred_at, received_at) ON TABLE {{table:platform.mail_feedback}} TO {{role:report_ro}};
