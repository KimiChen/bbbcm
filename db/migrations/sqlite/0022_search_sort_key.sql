-- 把排序键物化进检索文档表。
--
-- 分档实测（`99` §5.39）：十万篇语料下命中 1,000 条时 p95 1395ms，超 400ms 门槛
-- 3.5 倍——而命中 1,000 条是完全普通的搜索结果规模。根因不是缺索引（GIN 已建），
-- 而是排序键 `created_at` 在 `topics` 上：查询必须先取回全部匹配、与 `topics`
-- 连接、再全量排序才能取前 50，排序无法提前截断。
--
-- 物化之后连接消失，且 `(created_at DESC, topic_id DESC)` 上的有序扫描可以在凑满
-- 50 条后停下。低命中率仍走 GIN，高命中率走有序扫描，由规划器按选择性各取所需。
--
-- `search_documents` 本就冗余了 title、body、visibility、forum_id，多一列排序键
-- 与既有设计一致；代价是它必须与主题的 created_at 保持同步，由 `rebuild_search`
-- 的同一条写路径负责。
ALTER TABLE {{table:community.search_documents}} ADD COLUMN created_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00.000000Z';
CREATE INDEX {{index:community.search_sort_idx}} ON {{table:community.search_documents}} (created_at, topic_id);
