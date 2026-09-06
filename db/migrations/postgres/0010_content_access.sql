-- The constraint name is PostgreSQL's fixed name from 0007, scoped by the rendered table.
ALTER TABLE {{table:trust.risk_events}} DROP CONSTRAINT risk_events_event_kind_check;
ALTER TABLE {{table:trust.risk_events}} ADD CONSTRAINT risk_events_event_kind_check CHECK (event_kind IN ('registration', 'login', 'invitation', 'topic', 'reply', 'edit', 'delete'));
CREATE TABLE {{table:moderation.content_restrictions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    capability TEXT NOT NULL CHECK (capability IN ('content', 'topic', 'reply', 'edit', 'delete')),
    starts_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked BOOLEAN NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    CHECK (expires_at > starts_at)
);
CREATE INDEX {{index:moderation.content_restriction_owner_idx}} ON {{table:moderation.content_restrictions}} (user_id, revoked, starts_at, expires_at);
REVOKE ALL ON TABLE {{table:moderation.content_restrictions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:moderation.content_restrictions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.content_restrictions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
