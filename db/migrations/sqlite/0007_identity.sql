-- B01/B02/B04 identity contracts. No accounts, privileges, policies or enabled flags are seeded.
CREATE TABLE {{table:identity.users}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    public_id TEXT NOT NULL UNIQUE,
    analytics_pseudonym TEXT NOT NULL UNIQUE,
    analytics_excluded INTEGER NOT NULL CHECK (analytics_excluded IN (0, 1)),
    username TEXT NOT NULL,
    username_key TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL,
    email_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('active', 'locked', 'deleted')),
    email_verified INTEGER NOT NULL CHECK (email_verified IN (0, 1)),
    bio TEXT NOT NULL,
    created_at TEXT NOT NULL,
    username_changed_at TEXT NOT NULL,
    credential_version BIGINT NOT NULL CHECK (credential_version > 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:identity.credentials}} (
    user_id BIGINT PRIMARY KEY NOT NULL REFERENCES {{table:identity.users}} (id),
    password_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE {{table:identity.sessions}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_ref TEXT NOT NULL UNIQUE,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    token_hash TEXT NOT NULL UNIQUE,
    csrf_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    absolute_expires_at TEXT NOT NULL,
    idle_expires_at TEXT NOT NULL,
    reauthenticated_at TEXT NOT NULL,
    credential_version BIGINT NOT NULL CHECK (credential_version > 0),
    device_summary TEXT NOT NULL,
    revoked INTEGER NOT NULL CHECK (revoked IN (0, 1))
);
CREATE TABLE {{table:identity.one_time_tokens}} (
    token_id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    issued_credential_version BIGINT NOT NULL CHECK (issued_credential_version > 0),
    purpose TEXT NOT NULL CHECK (purpose IN ('verify_email', 'reset_password', 'change_email')),
    token_hash TEXT NOT NULL UNIQUE,
    target_email TEXT NOT NULL,
    target_email_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed INTEGER NOT NULL CHECK (consumed IN (0, 1)),
    key_version TEXT NOT NULL,
    token_nonce TEXT NOT NULL
);
CREATE TABLE {{table:identity.invitations}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invite_ref TEXT NOT NULL UNIQUE,
    creator_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    code_hash TEXT NOT NULL UNIQUE,
    source TEXT NOT NULL CHECK (source IN ('user', 'admin')),
    state TEXT NOT NULL CHECK (state IN ('available', 'used', 'revoked', 'frozen')),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT NOT NULL,
    consumer_id BIGINT NOT NULL CHECK (consumer_id >= 0)
);
CREATE TABLE {{table:identity.policy_consents}} (
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    kind TEXT NOT NULL CHECK (kind IN ('terms', 'privacy', 'community')),
    version_ref TEXT NOT NULL,
    accepted_at TEXT NOT NULL,
    PRIMARY KEY (user_id, kind, version_ref)
);
CREATE TABLE {{table:identity.username_claims}} (
    username_key TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    kind TEXT NOT NULL CHECK (kind IN ('current', 'history')),
    reserved_until TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:identity.username_history}} (
    username_key TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    released_at TEXT NOT NULL,
    reserved_until TEXT NOT NULL,
    PRIMARY KEY (username_key, user_id, released_at)
);
CREATE TABLE {{table:identity.mail_requests}} (
    message_id TEXT PRIMARY KEY NOT NULL,
    user_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    token_id TEXT NOT NULL,
    recipient_email TEXT NOT NULL,
    recipient_ref TEXT NOT NULL,
    kind TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE {{table:identity.request_principals}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token_hash TEXT NOT NULL UNIQUE,
    csrf_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE TABLE {{table:trust.identity_snapshots}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.users}} (id),
    level TEXT NOT NULL CHECK (level IN ('pending', 't0', 't1')),
    email_verified INTEGER NOT NULL CHECK (email_verified IN (0, 1)),
    invitation_allowed INTEGER NOT NULL CHECK (invitation_allowed IN (0, 1)),
    policy_version BIGINT NOT NULL CHECK (policy_version >= 0),
    reason_code TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:trust.invite_uses}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invitation_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.invitations}} (id),
    inviter_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id),
    invitee_id BIGINT NOT NULL UNIQUE REFERENCES {{table:identity.users}} (id),
    registered_at TEXT NOT NULL,
    email_verified INTEGER NOT NULL CHECK (email_verified IN (0, 1)),
    updated_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:trust.risk_events}} (
    event_version BIGINT NOT NULL CHECK (event_version = 1),
    event_id TEXT PRIMARY KEY NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('registration', 'login', 'invitation')),
    actor_id BIGINT NOT NULL CHECK (actor_id >= 0),
    occurred_at TEXT NOT NULL,
    rule_version BIGINT NOT NULL CHECK (rule_version >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('allow', 'deny', 'challenge', 'unavailable')),
    reason_code TEXT NOT NULL,
    signals_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL
);
CREATE TABLE {{table:moderation.policy_versions}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('terms', 'privacy', 'community')),
    version BIGINT NOT NULL CHECK (version > 0),
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('synthetic', 'published', 'retired')),
    created_at TEXT NOT NULL,
    UNIQUE (kind, version)
);
CREATE INDEX {{index:identity.session_owner_idx}} ON {{table:identity.sessions}} (user_id, revoked, id);
CREATE INDEX {{index:identity.invite_owner_idx}} ON {{table:identity.invitations}} (creator_id, created_at, id);
CREATE INDEX {{index:identity.token_owner_idx}} ON {{table:identity.one_time_tokens}} (user_id, purpose, consumed);
CREATE INDEX {{index:trust.risk_actor_idx}} ON {{table:trust.risk_events}} (actor_id, occurred_at);
CREATE TABLE {{table:identity.mail_recipients}} (
    address_hash TEXT PRIMARY KEY NOT NULL,
    recipient_ref TEXT UNIQUE NOT NULL
);
