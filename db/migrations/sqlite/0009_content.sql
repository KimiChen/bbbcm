-- C02 content and C05 authority projection. No publication policy, accounts or enabled flags are seeded.
ALTER TABLE {{table:community.forums}} ADD COLUMN parent_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE {{table:community.forums}} ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE {{table:community.forums}} ADD COLUMN icon TEXT NOT NULL DEFAULT '';
ALTER TABLE {{table:community.forums}} ADD COLUMN visibility TEXT NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'members'));
CREATE TABLE {{table:community.commit_state}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id = 1),
    sequence_no INTEGER NOT NULL CHECK (sequence_no >= 0),
    version INTEGER NOT NULL CHECK (version > 0)
);
INSERT INTO {{table:community.commit_state}} (sequence_no, version) VALUES (0, 1);
CREATE TABLE {{table:community.tags}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT NOT NULL COLLATE BINARY UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    restricted INTEGER NOT NULL CHECK (restricted IN (0, 1)),
    merged_into INTEGER NOT NULL CHECK (merged_into >= 0),
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:community.tag_aliases}} (
    alias TEXT PRIMARY KEY NOT NULL COLLATE BINARY,
    tag_id INTEGER NOT NULL REFERENCES {{table:community.tags}} (id) ON DELETE RESTRICT
);
CREATE TABLE {{table:community.topics}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    forum_id INTEGER NOT NULL REFERENCES {{table:community.forums}} (id) ON DELETE RESTRICT,
    author_id INTEGER NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (author_id > 0),
    title TEXT NOT NULL,
    publication TEXT NOT NULL CHECK (publication IN ('draft', 'pending', 'published')),
    moderation TEXT NOT NULL CHECK (moderation IN ('normal', 'obscured', 'hidden')),
    locked INTEGER NOT NULL CHECK (locked IN (0, 1)),
    visibility TEXT NOT NULL CHECK (visibility IN ('public', 'members')),
    deleted_at TEXT NOT NULL,
    archived_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    edited_at TEXT NOT NULL,
    create_seq INTEGER NOT NULL CHECK (create_seq > 0),
    current_seq INTEGER NOT NULL CHECK (current_seq > 0),
    last_reply_seq INTEGER NOT NULL CHECK (last_reply_seq > 0),
    next_floor INTEGER NOT NULL CHECK (next_floor >= 2),
    raw_reply_count INTEGER NOT NULL CHECK (raw_reply_count >= 0),
    last_reply_author_id INTEGER NOT NULL CHECK (last_reply_author_id >= 0),
    last_reply_at TEXT NOT NULL,
    pinned INTEGER NOT NULL CHECK (pinned IN (0, 1)),
    globally_pinned INTEGER NOT NULL CHECK (globally_pinned IN (0, 1)),
    featured INTEGER NOT NULL CHECK (featured IN (0, 1)),
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:community.posts}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    topic_id INTEGER NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    author_id INTEGER NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (author_id > 0),
    floor_no INTEGER NOT NULL CHECK (floor_no > 0),
    body TEXT NOT NULL,
    quote_post_id INTEGER NOT NULL CHECK (quote_post_id >= 0),
    publication TEXT NOT NULL CHECK (publication IN ('draft', 'pending', 'published')),
    moderation TEXT NOT NULL CHECK (moderation IN ('normal', 'obscured', 'hidden')),
    deleted_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    edited_at TEXT NOT NULL,
    create_seq INTEGER NOT NULL CHECK (create_seq > 0),
    version INTEGER NOT NULL CHECK (version > 0),
    UNIQUE (topic_id, floor_no)
);
CREATE TABLE {{table:community.post_revisions}} (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    revision_no INTEGER NOT NULL CHECK (revision_no > 0),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    state_json TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    editor_id INTEGER NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (editor_id > 0),
    created_at TEXT NOT NULL,
    UNIQUE (post_id, revision_no)
);
CREATE TABLE {{table:community.topic_tags}} (
    topic_id INTEGER NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    tag_id INTEGER NOT NULL REFERENCES {{table:community.tags}} (id) ON DELETE RESTRICT,
    PRIMARY KEY (topic_id, tag_id)
);
CREATE TABLE {{table:community.topic_history}} (
    sequence_no INTEGER NOT NULL CHECK (sequence_no > 0),
    topic_id INTEGER NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    create_seq INTEGER NOT NULL CHECK (create_seq > 0),
    last_reply_seq INTEGER NOT NULL CHECK (last_reply_seq > 0),
    pinned INTEGER NOT NULL CHECK (pinned IN (0, 1)),
    globally_pinned INTEGER NOT NULL CHECK (globally_pinned IN (0, 1)),
    featured INTEGER NOT NULL CHECK (featured IN (0, 1)),
    PRIMARY KEY (sequence_no, topic_id)
);
CREATE TABLE {{table:community.publication_receipts}} (
    operation_id TEXT PRIMARY KEY NOT NULL,
    actor_id INTEGER NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (actor_id > 0),
    action TEXT NOT NULL CHECK (action IN ('create_topic', 'create_reply')),
    post_id INTEGER NOT NULL UNIQUE REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    occurred_at TEXT NOT NULL,
    charged INTEGER NOT NULL CHECK (charged IN (0, 1)),
    policy_version INTEGER NOT NULL CHECK (policy_version > 0)
);
CREATE TABLE {{table:community.search_documents}} (
    post_id INTEGER PRIMARY KEY NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    topic_id INTEGER NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    forum_id INTEGER NOT NULL REFERENCES {{table:community.forums}} (id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    body TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('public', 'members')),
    forum_visibility TEXT NOT NULL CHECK (forum_visibility IN ('public', 'members')),
    publication TEXT NOT NULL CHECK (publication IN ('draft', 'pending', 'published')),
    moderation TEXT NOT NULL CHECK (moderation IN ('normal', 'obscured', 'hidden')),
    deleted_at TEXT NOT NULL,
    entity_version INTEGER NOT NULL CHECK (entity_version > 0),
    topic_version INTEGER NOT NULL CHECK (topic_version > 0),
    create_seq INTEGER NOT NULL CHECK (create_seq > 0)
);
CREATE TABLE {{table:community.post_links}} (
    post_id INTEGER NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    target_post_id INTEGER NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    PRIMARY KEY (post_id, target_post_id)
);
CREATE INDEX {{index:community.topic_forum_idx}} ON {{table:community.topics}} (forum_id, create_seq, id);
CREATE INDEX {{index:community.topic_owner_idx}} ON {{table:community.topics}} (author_id, create_seq, id);
CREATE INDEX {{index:community.post_floor_idx}} ON {{table:community.posts}} (topic_id, floor_no, create_seq);
CREATE INDEX {{index:community.post_owner_idx}} ON {{table:community.posts}} (author_id, create_seq, id);
CREATE INDEX {{index:community.history_topic_idx}} ON {{table:community.topic_history}} (topic_id, sequence_no);
CREATE INDEX {{index:community.publication_owner_idx}} ON {{table:community.publication_receipts}} (actor_id, action, occurred_at);
CREATE INDEX {{index:community.search_topic_idx}} ON {{table:community.search_documents}} (topic_id, post_id);
