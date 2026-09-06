-- DEC-051: local challenge state only; no provider, account, role or enabled flag seed.
CREATE TABLE {{table:identity.challenges}} (
    challenge_id TEXT PRIMARY KEY NOT NULL,
    principal_id BIGINT NOT NULL REFERENCES {{table:identity.request_principals}} (id),
    action TEXT NOT NULL CHECK (action IN ('register', 'login', 'recovery', 'verification_resend')),
    request_hash TEXT NOT NULL,
    proof_hash TEXT NOT NULL,
    nonce TEXT NOT NULL,
    key_version TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed INTEGER NOT NULL CHECK (consumed IN (0, 1)),
    UNIQUE (principal_id, action, request_hash)
);
