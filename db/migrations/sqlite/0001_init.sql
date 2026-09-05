-- A03 uses the same logical domains; SQLite has flat tables and no PostgreSQL role emulation.
CREATE TABLE {{table:community.forums}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT NOT NULL COLLATE BINARY UNIQUE,
    name TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position > 0),
    status TEXT NOT NULL CHECK (status IN ('draft', 'active', 'closed')),
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:moderation.rule_versions}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    forum_id INTEGER NOT NULL REFERENCES {{table:community.forums}} (id) ON DELETE RESTRICT,
    version INTEGER NOT NULL CHECK (version > 0),
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('placeholder', 'published', 'retired')),
    UNIQUE (forum_id, version)
);
