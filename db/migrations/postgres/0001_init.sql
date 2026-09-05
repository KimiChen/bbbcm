-- A03: initial objects only; application engines arrive in their own work packages.
CREATE SCHEMA IF NOT EXISTS {{schema:platform}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:platform}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:identity}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:identity}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:iam}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:iam}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:community}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:community}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:media}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:media}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:interaction}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:interaction}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:trust}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:trust}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:campaign}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:campaign}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:commerce}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:commerce}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:moderation}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:moderation}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:support}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:support}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:privacy}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:privacy}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:operations}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:operations}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:audit}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:audit}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE SCHEMA IF NOT EXISTS {{schema:analytics}} AUTHORIZATION {{role:migrator}};
REVOKE ALL ON SCHEMA {{schema:analytics}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
CREATE TABLE {{table:community.forums}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position > 0),
    status TEXT NOT NULL CHECK (status IN ('draft', 'active', 'closed')),
    version BIGINT NOT NULL CHECK (version > 0)
);
CREATE TABLE {{table:moderation.rule_versions}} (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    forum_id BIGINT NOT NULL REFERENCES {{table:community.forums}} (id) ON DELETE RESTRICT,
    version INTEGER NOT NULL CHECK (version > 0),
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('placeholder', 'published', 'retired')),
    UNIQUE (forum_id, version)
);
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} REVOKE ALL ON SCHEMAS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:platform}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:platform}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:platform}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:platform}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:identity}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:identity}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:identity}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:identity}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:iam}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:iam}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:iam}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:iam}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:community}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:community}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:community}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:community}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:media}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:media}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:media}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:media}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:interaction}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:interaction}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:interaction}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:interaction}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:trust}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:trust}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:trust}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:trust}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:campaign}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:campaign}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:campaign}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:campaign}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:commerce}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:commerce}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:commerce}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:commerce}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:moderation}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:moderation}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:moderation}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:moderation}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:support}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:support}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:support}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:support}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:privacy}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:privacy}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:privacy}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:privacy}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:operations}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:operations}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:operations}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:operations}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:audit}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:audit}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:audit}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:audit}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:analytics}} REVOKE ALL ON TABLES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:analytics}} REVOKE ALL ON SEQUENCES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:analytics}} REVOKE ALL ON FUNCTIONS FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
ALTER DEFAULT PRIVILEGES FOR ROLE {{role:migrator}} IN SCHEMA {{schema:analytics}} REVOKE ALL ON TYPES FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON TABLE {{table:platform.schema_migrations}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON TABLE {{table:community.forums}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON TABLE {{table:moderation.rule_versions}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON SEQUENCE {{sequence:community.forums.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
REVOKE ALL ON SEQUENCE {{sequence:moderation.rule_versions.id}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
-- A03 exposes no domain writes. Current web policy reads only the two non-sensitive skeleton tables.
GRANT USAGE ON SCHEMA {{schema:community}}, {{schema:moderation}} TO {{role:app_rw}};
GRANT SELECT ON TABLE {{table:community.forums}}, {{table:moderation.rule_versions}} TO {{role:app_rw}};
-- worker_rw and report_ro have no A03 table or sequence grants; future grants must be explicit.
