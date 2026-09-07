-- 把 tsvector 物化成列，让 GIN 查找与过滤判定读同一份预算好的向量。
--
-- 为什么需要这一步（`99` §5.40 实测）：0022 把排序键物化后，1% 命中档走
-- `search_sort_idx` 有序扫描，`Rows Removed by Filter: 12865`（每 worker），
-- **每扫一行都要重新对正文做一次 jieba 分词**再判 `@@`。表达式 GIN 索引只能加速
-- 索引查找，免不了过滤路径上的重算。物化成列后两条计划都不再分词。
--
-- 用 GENERATED ALWAYS ... STORED 而不是普通列 + 写路径维护：普通列一旦漏写就
-- 变成静默的检索缺漏——文档在库里、搜不到，且没有任何报错。生成列由数据库保证
-- 与 title/body 同步，不存在漏写。两参数形式的 to_tsvector(regconfig, text) 是
-- IMMUTABLE，满足生成列的要求。
--
-- 合并 title 与 body 成一个向量，与原查询的 `title @@ q OR body @@ q` 等价：
-- 命中任一列即命中合并向量。合并后只需一个索引、一个判定。
ALTER TABLE {{table:community.search_documents}}
  ADD COLUMN search_vector tsvector
  GENERATED ALWAYS AS (to_tsvector('public.jiebaqry', coalesce(title, '') || ' ' || coalesce(body, ''))) STORED;
CREATE INDEX {{index:community.search_vector_gin_idx}} ON {{table:community.search_documents}} USING gin (search_vector);
-- 两个表达式索引自此无用：查询改判 search_vector，它们只会拖慢写入。
--
-- **必须限定 schema**：索引模板在 PostgreSQL 上只渲染裸索引名（索引名在
-- schema 内唯一，CREATE INDEX 由表决定归属，无需限定），而运行连接的
-- `search_path` 被 `prepare_postgres` 固定为 `pg_catalog`，裸名的 DROP INDEX
-- 找不到对象。实测报错 `index "search_title_gin_idx" does not exist`。
DROP INDEX {{schema:community}}.{{index:community.search_title_gin_idx}};
DROP INDEX {{schema:community}}.{{index:community.search_body_gin_idx}};
