-- DEC-051: local challenge state only; no provider, account, role or enabled flag seed.
CREATE TABLE {{table:identity.challenges}} (
    challenge_id TEXT PRIMARY KEY,
    principal_id BIGINT NOT NULL REFERENCES {{table:identity.request_principals}} (id),
    action TEXT NOT NULL CHECK (action IN ('register', 'login', 'recovery', 'verification_resend')),
    request_hash TEXT NOT NULL,
    proof_hash TEXT NOT NULL,
    nonce TEXT NOT NULL,
    key_version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed BOOLEAN NOT NULL,
    UNIQUE (principal_id, action, request_hash)
);
REVOKE ALL ON TABLE {{table:identity.challenges}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:identity.challenges}} TO {{role:app_rw}};
