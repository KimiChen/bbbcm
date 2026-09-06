-- C02 content and C05 authority projection. No publication policy, accounts or enabled flags are seeded.
ALTER TABLE {{table:community.forums}} ADD COLUMN parent_id BIGINT NOT NULL DEFAULT 0;
ALTER TABLE {{table:community.forums}} ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE {{table:community.forums}} ADD COLUMN icon TEXT NOT NULL DEFAULT '';
ALTER TABLE {{table:community.forums}} ADD COLUMN visibility TEXT NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'members'));
CREATE TABLE {{table:community.commit_state}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY CHECK (id = 1),
    sequence_no BIGINT NOT NULL CHECK (sequence_no >= 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
INSERT INTO {{table:community.commit_state}} (sequence_no, version) VALUES (0, 1);
CREATE TABLE {{table:community.tags}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    restricted BOOLEAN NOT NULL,
    merged_into BIGINT NOT NULL CHECK (merged_into >= 0),
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:community.tag_aliases}} (
    alias TEXT PRIMARY KEY NOT NULL,
    tag_id BIGINT NOT NULL REFERENCES {{table:community.tags}} (id) ON DELETE RESTRICT
);
CREATE TABLE {{table:community.topics}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    forum_id BIGINT NOT NULL REFERENCES {{table:community.forums}} (id) ON DELETE RESTRICT,
    author_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (author_id > 0),
    title TEXT NOT NULL,
    publication TEXT NOT NULL CHECK (publication IN ('draft', 'pending', 'published')),
    moderation TEXT NOT NULL CHECK (moderation IN ('normal', 'obscured', 'hidden')),
    locked BOOLEAN NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('public', 'members')),
    deleted_at TIMESTAMPTZ NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    edited_at TIMESTAMPTZ NOT NULL,
    create_seq BIGINT NOT NULL CHECK (create_seq > 0),
    current_seq BIGINT NOT NULL CHECK (current_seq > 0),
    last_reply_seq BIGINT NOT NULL CHECK (last_reply_seq > 0),
    next_floor BIGINT NOT NULL CHECK (next_floor >= 2),
    raw_reply_count BIGINT NOT NULL CHECK (raw_reply_count >= 0),
    last_reply_author_id BIGINT NOT NULL CHECK (last_reply_author_id >= 0),
    last_reply_at TIMESTAMPTZ NOT NULL,
    pinned BOOLEAN NOT NULL,
    globally_pinned BOOLEAN NOT NULL,
    featured BOOLEAN NOT NULL,
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:community.posts}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    topic_id BIGINT NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    author_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (author_id > 0),
    floor_no BIGINT NOT NULL CHECK (floor_no > 0),
    body TEXT NOT NULL,
    quote_post_id BIGINT NOT NULL CHECK (quote_post_id >= 0),
    publication TEXT NOT NULL CHECK (publication IN ('draft', 'pending', 'published')),
    moderation TEXT NOT NULL CHECK (moderation IN ('normal', 'obscured', 'hidden')),
    deleted_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    edited_at TIMESTAMPTZ NOT NULL,
    create_seq BIGINT NOT NULL CHECK (create_seq > 0),
    version BIGINT NOT NULL CHECK (version > 0),
    UNIQUE (topic_id, floor_no)
);
CREATE TABLE {{table:community.post_revisions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    post_id BIGINT NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    revision_no BIGINT NOT NULL CHECK (revision_no > 0),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    state_json TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    editor_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (editor_id > 0),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (post_id, revision_no)
);
CREATE TABLE {{table:community.topic_tags}} (
    topic_id BIGINT NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    tag_id BIGINT NOT NULL REFERENCES {{table:community.tags}} (id) ON DELETE RESTRICT,
    PRIMARY KEY (topic_id, tag_id)
);
CREATE TABLE {{table:community.topic_history}} (
    sequence_no BIGINT NOT NULL CHECK (sequence_no > 0),
    topic_id BIGINT NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    create_seq BIGINT NOT NULL CHECK (create_seq > 0),
    last_reply_seq BIGINT NOT NULL CHECK (last_reply_seq > 0),
    pinned BOOLEAN NOT NULL,
    globally_pinned BOOLEAN NOT NULL,
    featured BOOLEAN NOT NULL,
    PRIMARY KEY (sequence_no, topic_id)
);
CREATE TABLE {{table:community.publication_receipts}} (
    operation_id TEXT PRIMARY KEY NOT NULL,
    actor_id BIGINT NOT NULL REFERENCES {{table:identity.users}} (id) ON DELETE RESTRICT CHECK (actor_id > 0),
    action TEXT NOT NULL CHECK (action IN ('create_topic', 'create_reply')),
    post_id BIGINT NOT NULL UNIQUE REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    occurred_at TIMESTAMPTZ NOT NULL,
    charged BOOLEAN NOT NULL,
    policy_version BIGINT NOT NULL CHECK (policy_version > 0)
);
CREATE TABLE {{table:community.search_documents}} (
    post_id BIGINT PRIMARY KEY NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    topic_id BIGINT NOT NULL REFERENCES {{table:community.topics}} (id) ON DELETE RESTRICT,
    forum_id BIGINT NOT NULL REFERENCES {{table:community.forums}} (id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    body TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('public', 'members')),
    forum_visibility TEXT NOT NULL CHECK (forum_visibility IN ('public', 'members')),
    publication TEXT NOT NULL CHECK (publication IN ('draft', 'pending', 'published')),
    moderation TEXT NOT NULL CHECK (moderation IN ('normal', 'obscured', 'hidden')),
    deleted_at TIMESTAMPTZ NOT NULL,
    entity_version BIGINT NOT NULL CHECK (entity_version > 0),
    topic_version BIGINT NOT NULL CHECK (topic_version > 0),
    create_seq BIGINT NOT NULL CHECK (create_seq > 0)
);
CREATE TABLE {{table:community.post_links}} (
    post_id BIGINT NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    target_post_id BIGINT NOT NULL REFERENCES {{table:community.posts}} (id) ON DELETE RESTRICT,
    PRIMARY KEY (post_id, target_post_id)
);
CREATE INDEX {{index:community.topic_forum_idx}} ON {{table:community.topics}} (forum_id, create_seq, id);
CREATE INDEX {{index:community.topic_owner_idx}} ON {{table:community.topics}} (author_id, create_seq, id);
CREATE INDEX {{index:community.post_floor_idx}} ON {{table:community.posts}} (topic_id, floor_no, create_seq);
CREATE INDEX {{index:community.post_owner_idx}} ON {{table:community.posts}} (author_id, create_seq, id);
CREATE INDEX {{index:community.history_topic_idx}} ON {{table:community.topic_history}} (topic_id, sequence_no);
CREATE INDEX {{index:community.publication_owner_idx}} ON {{table:community.publication_receipts}} (actor_id, action, occurred_at);
CREATE INDEX {{index:community.search_topic_idx}} ON {{table:community.search_documents}} (topic_id, post_id);
GRANT USAGE ON SCHEMA {{schema:community}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.commit_state}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, UPDATE ON TABLE {{table:community.commit_state}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:community.commit_state.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:community.commit_state.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.tags}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:community.tags}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:community.tags.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON TABLE {{table:community.tag_aliases}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT ON TABLE {{table:community.tag_aliases}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.topics}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:community.topics}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:community.topics.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:community.topics.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.posts}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:community.posts}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:community.posts.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:community.posts.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.post_revisions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:community.post_revisions}} TO {{role:app_rw}};
REVOKE ALL ON SEQUENCE {{sequence:community.post_revisions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT USAGE, SELECT ON SEQUENCE {{sequence:community.post_revisions.id}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.topic_tags}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {{table:community.topic_tags}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.topic_history}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:community.topic_history}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.publication_receipts}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT ON TABLE {{table:community.publication_receipts}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.search_documents}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE ON TABLE {{table:community.search_documents}} TO {{role:app_rw}};
REVOKE ALL ON TABLE {{table:community.post_links}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {{table:community.post_links}} TO {{role:app_rw}};
