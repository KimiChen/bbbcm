-- 完整模式搜索索引。本轨与常规迁移轨分开执行，理由见 `02` §6.6-A。
--
-- 表达式必须与 `platform::db::full_text_search_sql` 生成的条件**逐字一致**，
-- 否则规划器不会选中索引，加了也等于没加：
--   to_tsvector('public.jiebaqry', d.title) @@ plainto_tsquery('public.jiebacfg', ?)
--   to_tsvector('public.jiebaqry', d.body)  @@ plainto_tsquery('public.jiebacfg', ?)
-- 两个条件是 OR 关系、分列两个字段，因此建两个独立表达式索引；合并成一个
-- to_tsvector(title || ' ' || body) 无法服务于只命中其中一列的查询。
--
-- 配置名写全限定 `public.jiebaqry`：表达式索引要求函数不可变，两参数形式的
-- to_tsvector(regconfig, text) 才是 IMMUTABLE；依赖 search_path 的单参数形式
-- 是 STABLE，建索引会直接失败。
CREATE INDEX {{index:community.search_title_gin_idx}} ON {{table:community.search_documents}} USING gin (to_tsvector('public.jiebaqry', title));
CREATE INDEX {{index:community.search_body_gin_idx}} ON {{table:community.search_documents}} USING gin (to_tsvector('public.jiebaqry', body));
-- 本轨的历史表由 `search_index::bootstrap` 在执行本文件之前建好，这里补显式 ACL，
-- 让「每张登记表都有显式 REVOKE」这条不变量在搜索轨同样成立（`acl.rs` 覆盖测试）。
REVOKE ALL ON TABLE {{table:platform.search_migrations}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};
