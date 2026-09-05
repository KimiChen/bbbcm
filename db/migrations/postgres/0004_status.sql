-- Only reviewed C0 announcements. Runtime has no announcement write entry.
GRANT USAGE ON SCHEMA {{schema:operations}} TO {{role:app_rw}};
CREATE TABLE {{table:operations.status_announcements}} (
    announcement_id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('maintenance', 'incident')),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    published BOOLEAN NOT NULL,
    published_at TIMESTAMPTZ NOT NULL,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
REVOKE ALL ON TABLE {{table:operations.status_announcements}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:operations.status_announcements}} TO {{role:app_rw}};
