-- A04 engine only: no identity, content, approval or analytics business data.
CREATE TABLE {{table:platform.idempotency_records}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL,
    actor_id BIGINT NOT NULL CHECK (actor_id > 0),
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    operation_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('pending', 'completed')),
    response_status INTEGER NOT NULL DEFAULT 0,
    response_json TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    UNIQUE (scope, actor_id, idempotency_key)
);
CREATE TABLE {{table:platform.outbox_events}} (
    event_id TEXT PRIMARY KEY NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    occurred_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('ready', 'leased', 'delivered', 'dlq')),
    attempts BIGINT NOT NULL CHECK (attempts >= 0),
    available_at TEXT NOT NULL,
    lease_owner TEXT NOT NULL,
    lease_batch_id TEXT NOT NULL,
    lease_until TEXT NOT NULL,
    error_code TEXT NOT NULL,
    replay_count BIGINT NOT NULL CHECK (replay_count >= 0)
);
CREATE INDEX {{index:platform.outbox_ready_idx}} ON {{table:platform.outbox_events}} (state, available_at, event_id);
CREATE INDEX {{index:platform.outbox_lease_idx}} ON {{table:platform.outbox_events}} (state, lease_until, event_id);
CREATE INDEX {{index:platform.outbox_batch_idx}} ON {{table:platform.outbox_events}} (lease_batch_id, event_id);
CREATE TABLE {{table:platform.sink_deliveries}} (
    sink TEXT NOT NULL,
    event_id TEXT NOT NULL REFERENCES {{table:platform.outbox_events}} (event_id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('pending', 'delivered')),
    delivered_at TEXT NOT NULL,
    PRIMARY KEY (sink, event_id)
);
CREATE INDEX {{index:platform.sink_event_idx}} ON {{table:platform.sink_deliveries}} (event_id, sink);
CREATE TABLE {{table:audit.audit_events}} (
    event_id TEXT PRIMARY KEY NOT NULL,
    actor_id BIGINT NOT NULL CHECK (actor_id > 0),
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    ticket_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    result TEXT NOT NULL CHECK (result IN ('succeeded', 'denied', 'failed')),
    operation_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    details TEXT NOT NULL,
    correction_of TEXT NOT NULL
);
CREATE INDEX {{index:audit.audit_actor_idx}} ON {{table:audit.audit_events}} (actor_id, occurred_at, event_id);
CREATE TABLE {{table:audit.sensitive_access_events}} (
    event_id TEXT PRIMARY KEY NOT NULL,
    actor_id BIGINT NOT NULL CHECK (actor_id > 0),
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    ticket_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    result TEXT NOT NULL CHECK (result IN ('succeeded', 'denied', 'failed')),
    operation_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0),
    details TEXT NOT NULL,
    correction_of TEXT NOT NULL
);
CREATE INDEX {{index:audit.access_actor_idx}} ON {{table:audit.sensitive_access_events}} (actor_id, occurred_at, event_id);
