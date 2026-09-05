-- B01/B02/B04 identity contracts. No accounts, privileges, policies or enabled flags are seeded.
GRANT USAGE ON SCHEMA {{schema:identity}}, {{schema:trust}} TO {{role:app_rw}};
GRANT USAGE ON SCHEMA {{schema:identity}} TO {{role:worker_rw}};
CREATE TABLE {{table:identity.users}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE,
    analytics_pseudonym TEXT NOT NULL UNIQUE,
    analytics_excluded BOOLEAN NOT NULL,
    username TEXT NOT NULL,
    username_key TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL,
    email_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('active', 'locked', 'deleted')),
    email_verified BOOLEAN NOT NULL,
    bio TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    username_changed_at TIMESTAMPTZ NOT NULL,
    credential_version BIGINT NOT NULL CHECK (credential_version > 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:identity.credentials}} (
    user_id BIGINT PRIMARY KEY NOT NULL REFERENCES {{table:identity.users}} (id),
    password_hash TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE {{table:identity.sessions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    session_ref TEXT NOT NULL UNIQUE,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    token_hash TEXT NOT NULL UNIQUE,
    csrf_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    absolute_expires_at TIMESTAMPTZ NOT NULL,
    idle_expires_at TIMESTAMPTZ NOT NULL,
    reauthenticated_at TIMESTAMPTZ NOT NULL,
    credential_version BIGINT NOT NULL CHECK (credential_version > 0),
    device_summary TEXT NOT NULL,
    revoked BOOLEAN NOT NULL
);
CREATE TABLE {{table:identity.one_time_tokens}} (
    token_id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    issued_credential_version BIGINT NOT NULL CHECK (issued_credential_version > 0),
    purpose TEXT NOT NULL CHECK (purpose IN ('verify_email', 'reset_password', 'change_email')),
    token_hash TEXT NOT NULL UNIQUE,
    target_email TEXT NOT NULL,
    target_email_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed BOOLEAN NOT NULL,
    key_version TEXT NOT NULL,
    token_nonce TEXT NOT NULL
);
CREATE TABLE {{table:identity.invitations}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    invite_ref TEXT NOT NULL UNIQUE,
    creator_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    code_hash TEXT NOT NULL UNIQUE,
    source TEXT NOT NULL CHECK (source IN ('user', 'admin')),
    state TEXT NOT NULL CHECK (state IN ('available', 'used', 'revoked', 'frozen')),
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ NOT NULL,
    consumer_id BIGINT NOT NULL CHECK (consumer_id >= 0)
);
CREATE TABLE {{table:identity.policy_consents}} (
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    kind TEXT NOT NULL CHECK (kind IN ('terms', 'privacy', 'community')),
    version_ref TEXT NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, kind, version_ref)
);
CREATE TABLE {{table:identity.username_claims}} (
    username_key TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    kind TEXT NOT NULL CHECK (kind IN ('current', 'history')),
    reserved_until TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:identity.username_history}} (
    username_key TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    released_at TIMESTAMPTZ NOT NULL,
    reserved_until TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (username_key, user_id, released_at)
);
CREATE TABLE {{table:identity.mail_requests}} (
    message_id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    token_id TEXT NOT NULL,
    recipient_email TEXT NOT NULL,
    recipient_ref TEXT NOT NULL,
    kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE {{table:identity.request_principals}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    csrf_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE {{table:trust.identity_snapshots}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.users}} (id),
    level TEXT NOT NULL CHECK (level IN ('pending', 't0', 't1')),
    email_verified BOOLEAN NOT NULL,
    invitation_allowed BOOLEAN NOT NULL,
    policy_version BIGINT NOT NULL CHECK (policy_version >= 0),
    reason_code TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:trust.invite_uses}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    invitation_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.invitations}} (id),
    inviter_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    invitee_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.users}} (id),
    registered_at TIMESTAMPTZ NOT NULL,
    email_verified BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:trust.risk_events}} (
    event_version BIGINT NOT NULL CHECK (event_version = 1),
    event_id TEXT PRIMARY KEY NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('registration', 'login', 'invitation')),
    actor_id BIGINT NOT NULL CHECK (actor_id >= 0),
    occurred_at TIMESTAMPTZ NOT NULL,
    rule_version BIGINT NOT NULL CHECK (rule_version >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'challenge', 'unavailable')),
    reason_code TEXT NOT NULL,
    signals_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL
);
CREATE TABLE {{table:moderation.policy_versions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('terms', 'privacy', 'community')),
    version BIGINT NOT NULL CHECK (version > 0),
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('synthetic', 'published', 'retired')),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (kind, version)
);
CREATE INDEX {{index:identity.session_owner_idx}} ON {{table:identity.sessions}} (user_id, revoked, id);
CREATE INDEX {{index:identity.invite_owner_idx}} ON {{table:identity.invitations}} (creator_id, created_at, id);
CREATE INDEX {{index:identity.token_owner_idx}} ON {{table:identity.one_time_tokens}} (user_id, purpose, consumed);
CREATE INDEX {{index:trust.risk_actor_idx}} ON {{table:trust.risk_events}} (actor_id, occurred_at);
REVOKE ALL ON TABLE {{table:identity.users}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.users}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.users.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.users.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.credentials}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.credentials}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.sessions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.sessions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.sessions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.sessions.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.one_time_tokens}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.one_time_tokens}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.invitations}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.invitations}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.invitations.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.invitations.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.policy_consents}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:identity.policy_consents}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.username_claims}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.username_claims}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.username_history}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:identity.username_history}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.mail_requests}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:identity.mail_requests}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:identity.request_principals}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.request_principals}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:identity.request_principals.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:identity.request_principals.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:trust.identity_snapshots}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:trust.identity_snapshots}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:trust.identity_snapshots.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:trust.identity_snapshots.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:trust.invite_uses}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:trust.invite_uses}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:trust.invite_uses.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:trust.invite_uses.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:trust.risk_events}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:trust.risk_events}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:moderation.policy_versions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:moderation.policy_versions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.policy_versions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:identity.mail_requests}} TO {{role:worker_rw}};
GRANT SELECT (token_id, user_id, issued_credential_version, purpose, token_hash, target_email, target_email_key, expires_at, key_version, token_nonce) ON TABLE {{table:identity.one_time_tokens}} TO {{role:worker_rw}};
CREATE TABLE {{table:identity.mail_recipients}} (
    address_hash TEXT PRIMARY KEY NOT NULL,
    recipient_ref TEXT UNIQUE NOT NULL
);
REVOKE ALL ON TABLE {{table:identity.mail_recipients}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:identity.mail_recipients}} TO {{role:app_rw}};
