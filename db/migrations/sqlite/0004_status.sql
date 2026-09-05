-- Only reviewed C0 announcements. Runtime has no announcement write entry.
CREATE TABLE {{table:operations.status_announcements}} (
    announcement_id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('maintenance', 'incident')),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    published INTEGER NOT NULL,
    published_at TEXT NOT NULL,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0)
);
