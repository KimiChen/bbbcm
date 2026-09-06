-- WP-MVP-D04 波次二：由 crates/manage/iam/registry.toml 生成的 IAM 目录种子。
-- 与 postgres 轨同编号配对，内容完全一致。
--
-- 本文件的内容必须与 registry.toml 及 build.rs 生成的 Rust 常量完全一致：
-- 启动时 manage::iam::verify_registry 会比对三者，任一不符即拒绝启动（CR-013）。
-- 因此修改角色或权限只能改 registry.toml，然后同步重新生成本迁移，
-- 绝不允许直接在这里手改一行。
--
-- 本迁移只写目录（有哪些角色、各持有哪些权限），不创建任何账号，
-- 也不给任何账号分配角色——首个站点所有者仍然只能走 AUTH-014 的一次性引导。
INSERT INTO {{table:iam.roles}} (role_key, name, version) VALUES ('site_owner', '站点所有者', 1);
INSERT INTO {{table:iam.roles}} (role_key, name, version) VALUES ('administrator', '管理员', 1);
INSERT INTO {{table:iam.roles}} (role_key, name, version) VALUES ('moderator', '版主', 1);
INSERT INTO {{table:iam.roles}} (role_key, name, version) VALUES ('global_moderator', '全局版主', 1);
INSERT INTO {{table:iam.roles}} (role_key, name, version) VALUES ('auditor', '只读审计员', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('administrator', 'community.content.move', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('administrator', 'moderation.appeals.review', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('administrator', 'moderation.cases.read', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('administrator', 'moderation.cases.write', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('administrator', 'moderation.enforcement.account', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('administrator', 'moderation.enforcement.content', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('auditor', 'moderation.cases.read', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('global_moderator', 'community.content.move', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('global_moderator', 'moderation.cases.read', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('global_moderator', 'moderation.cases.write', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('global_moderator', 'moderation.enforcement.account', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('global_moderator', 'moderation.enforcement.content', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('moderator', 'community.content.move', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('moderator', 'moderation.cases.read', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('moderator', 'moderation.cases.write', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('moderator', 'moderation.enforcement.content', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('site_owner', 'iam.billing.manage', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('site_owner', 'iam.disaster.recover', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('site_owner', 'iam.keys.rotate', 1);
INSERT INTO {{table:iam.role_permissions}} (role_key, permission_key, version) VALUES ('site_owner', 'iam.production.access', 1);
