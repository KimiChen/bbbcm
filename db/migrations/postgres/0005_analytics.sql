-- A06 committed analytics facts and rebuildable, pseudonym-free projections.
CREATE TABLE {{table:analytics.event_facts}} (
    event_id TEXT PRIMARY KEY NOT NULL,
    sink TEXT NOT NULL CHECK (sink = 'analytics.project'),
    dictionary_version BIGINT NOT NULL CHECK (dictionary_version = 1),
    event_kind TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL,
    fact_json TEXT NOT NULL,
    FOREIGN KEY (event_id) REFERENCES {{table:platform.outbox_events}} (event_id),
    FOREIGN KEY (sink, event_id) REFERENCES {{table:platform.sink_deliveries}} (sink, event_id)
);
CREATE TABLE {{table:analytics.projection_receipts}} (
    projection TEXT NOT NULL,
    event_id TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (projection, event_id),
    FOREIGN KEY (event_id) REFERENCES {{table:analytics.event_facts}} (event_id)
);
CREATE TABLE {{table:analytics.aggregates}} (
    projection TEXT NOT NULL,
    utc_day TIMESTAMPTZ NOT NULL,
    event_kind TEXT NOT NULL,
    dimension TEXT NOT NULL,
    dimension_value TEXT NOT NULL,
    event_count BIGINT NOT NULL CHECK (event_count > 0),
    version BIGINT NOT NULL CHECK (version > 0),
    PRIMARY KEY (projection, utc_day, event_kind, dimension, dimension_value)
);
CREATE TABLE {{table:analytics.watermarks}} (
    projection TEXT PRIMARY KEY NOT NULL,
    processed_count BIGINT NOT NULL CHECK (processed_count >= 0),
    excluded_count BIGINT NOT NULL CHECK (excluded_count >= 0),
    max_occurred_at TIMESTAMPTZ NOT NULL,
    max_received_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    maximum_delay_ms BIGINT NOT NULL CHECK (maximum_delay_ms >= 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
INSERT INTO {{table:analytics.watermarks}} (projection, processed_count, excluded_count, max_occurred_at, max_received_at, updated_at, maximum_delay_ms, version)
VALUES ('events.v1', 0, 0, '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z', 0, 1);
REVOKE ALL ON TABLE {{table:analytics.event_facts}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT INSERT ON TABLE {{table:analytics.event_facts}} TO {{role:app_rw}};
GRANT SELECT ON TABLE {{table:analytics.event_facts}} TO {{role:worker_rw}};
REVOKE ALL ON TABLE {{table:analytics.projection_receipts}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {{table:analytics.projection_receipts}} TO {{role:worker_rw}};
REVOKE ALL ON TABLE {{table:analytics.aggregates}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {{table:analytics.aggregates}} TO {{role:worker_rw}};
GRANT SELECT (projection, utc_day, event_kind, dimension, dimension_value, event_count) ON TABLE {{table:analytics.aggregates}} TO {{role:report_ro}};
REVOKE ALL ON TABLE {{table:analytics.watermarks}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {{table:analytics.watermarks}} TO {{role:worker_rw}};
GRANT USAGE ON SCHEMA {{schema:analytics}} TO {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
