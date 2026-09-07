# 草莓社区数据库重建方案：以 Discuz X3.5 为结构基准

记录日期：2026-09-07。

本文记录本轮源码研究、用户确认的设计方向及重建方案。[`README.md`](README.md) 仍是唯一计划入口，需求状态和证据仍统一登记在 [`99-traceability.md`](99-traceability.md)。本次仅保存方案，尚未完成 `01/02/03` 等正式事实源的同步，不在本文另设执行状态。实施前先按第四节同步正式决策、事实源、执行视图及相关本机规则；本文不表示现有数据库实现或阶段门禁已经完成变更。

## 一、总体方案与研究结论

按本轮确认，采用 **PostgreSQL 16 单库、Discuz 核心表结构尽量原样、现有产品功能继续保留** 的方案：

- 物理命名采用 `public.bcm_common_member`、`public.bcm_forum_thread` 等形式。
- 现有数据库基建整体替换，不导入旧数据、不保留兼容层或双写。
- 首页、版块、标签和搜索采用实时游标；排序变化可以使主题跨页移动，不再承诺一次遍历严格不重不漏。回复楼层永久稳定。
- Rust、Axum、Askama、sqlx、既有页面与产品需求继续保留。

### 1.1 研究对象与统计口径

研究对象为用户指定的本地安装包 `Discuz_X3.5_SC_UTF8_20260504/upload/`，依据其安装 SQL 与实际 PHP 查询代码，不以其他版本的资料替代。

| 基础安装 DDL | 物理表数 | 主键 | 唯一索引 | 普通索引 |
|---|---:|---:|---:|---:|
| `install/data/install.sql` | 258 | 231 | 23 | 326 |
| `uc_server/install/uc.sql` | 32 | 30 | 3 | 38 |
| 合计 | **290** | **261** | **26** | **364** |

主程序按表名前缀划分为 7 个模块：`common` 106 张、`forum` 95 张、`home` 34 张、`portal` 14 张、`connect` 5 张、`security` 3 张、`mobile` 1 张。统计包含安装 DDL 中的附件和私信分表，不包含运行时新增分表、插件表或升级中间结构。

主程序 258 张表均声明 InnoDB，没有显式外键，27 张没有显式主键。本项目采用其中的论坛结构，不引入门户、家园、插件及独立 UCenter 系统；沿用 `home_notification` 等表名不表示引入整个家园模块。

### 1.2 核心借鉴依据

| Discuz 实际设计 | 新设计采用方式 |
|---|---|
| `forum_thread` 保存列表元数据，`forum_post` 保存首帖与回复正文 | 保留，列表查询不读取正文 |
| 用户拆为 member、count、status、profile 等表 | 保留，按页面需要批量加载 |
| 版块列表索引 `(fid,displayorder,lastpost)` | 保留查询驱动思路，分别设计普通流和置顶查询 |
| 帖子主键 `(tid,position)`，`pid` 单独唯一 | 保留，支持永久楼层和帖子链接 |
| 作者、通知索引未完整覆盖时间排序 | 补齐排序列和唯一 ID |
| 附件分十表、帖子通过 `posttableid` 路由 | 首版使用单表，取消手工分表 |

主要源码依据如下，路径均相对于上述安装包的 `upload/`：

- `install/data/install.sql:698`：用户核心表及后续用户拆分表。
- `install/data/install.sql:2386`、`:2451`：版块及扩展表。
- `install/data/install.sql:2775`、`:2982`：帖子、主题及其实际索引。
- `source/class/table/table_forum_thread.php:495`：主题列表与首页缓存路径。
- `source/class/table/table_forum_post.php:77`：按楼层范围读取帖子。
- `source/module/forum/forum_viewthread.php:746`：详情页批量加载作者资料。
- `source/module/search/search_forum.php:337`：默认 SQL 搜索路径。

目前完成的是源码分析，尚未运行 Discuz 数据库进行 `EXPLAIN` 或性能测试。不能从索引定义直接推断实际吞吐，也不能把 InnoDB 的主键组织收益等同于 PostgreSQL 的物理存储行为。

## 二、数据表设计

以下表名统一加 `bcm_` 前缀，并放在 PostgreSQL 的 `public` Schema。表名示例用于设计说明；运行时标识符继续由受信注册表和已校验前缀生成，查询值一律使用 bind 参数。

| 数据组 | 核心表 | 设计约定 |
|---|---|---|
| 用户 | `common_member`、`common_member_count`、`common_member_status`、`common_member_profile`、`common_member_field_forum` | 保留 `uid`、`username`、`groupid`、`regdate` 等命名；公开资料、安全信息、统计分开读取 |
| 账号安全 | `common_member_security`、`common_session`，新增邀请、验证、MFA 表 | 单一账号体系；凭证哈希、密钥加密及会话撤销规则重新实现 |
| 版块 | `forum_forum`、`forum_forumfield`、`forum_access`、`forum_moderator` | 保留 `fid`、`fup`、名称、顺序、计数和扩展配置；权限关系结构化 |
| 主题 | `forum_thread` | 保留 `tid`、`fid`、`authorid`、`subject`、`dateline`、`lastpost`、`lastposter`、`views`、`replies`、`displayorder`、`digest`、`closed`、`maxposition` |
| 帖子 | `forum_post`，新增 `forum_post_revision` | 保留 `pid`、`tid`、`fid`、`first`、`authorid`、`message`、`position`、`repid`；修订独立、不可变 |
| 标签 | `common_tag`、`common_tagitem`，新增别名关系 | 保留 Discuz 关联方式；补齐别名、合并、受限标签要求 |
| 附件 | `forum_attachment`，新增引用、变体和处理任务表 | 不建 `_0…9`；上传隔离、扫描、引用、回收形成完整生命周期 |
| 互动 | `home_notification`、收藏、关注、反馈、阅读位置表 | 保留收件人、来源对象、时间等结构；添加唯一约束防重复 |
| 积分与信任 | `common_credit_rule`，新增账户、交易、分录和信任事件表 | 草莓使用追加式平衡账本；果农指数按既有规则计算，余额不能由普通日志替代 |
| 管理与平台 | `common_*` 下的权限、审批、审计、幂等、Outbox、任务及配置表 | 全部重新实现，承载现有后台与可靠性要求 |

“尽量原样”的必要调整固定如下：

- ID 使用 `BIGINT`；保留 `uid/fid/tid/pid` 字段名。时间字段保留 `dateline/lastpost` 名称，类型改为 `timestamptz`。对外 API 的资源 ID 继续使用十进制字符串。
- `forum_post` 保留 `PRIMARY KEY(tid,position)`、`UNIQUE(pid)`；首帖 `position=1`，后续楼层不重复、不回收。
- 保留 `displayorder` 的置顶含义，但不再混装待审、删除等状态。发布、审核、可见性、删除和归档采用独立字段。
- 核心关系建立外键和唯一约束；账号注销保留必要存根，禁止级联破坏帖子、账本和审计。
- 冗余计数、作者摘要和版块关联由事务维护；移帖、匿名化、删除与恢复同步处理。
- 不继承 PHP 序列化权限、旧密码方案、无主键表、分表发号表，以及没有对应产品功能的历史字段。
- 后续投票、悬赏、抽奖等能力按现有产品阶段建设主题扩展表，不将全部扩展字段堆入 `forum_thread`，也不因参考包已有对应表而提前开放功能。

## 三、查询、索引与事务

### 3.1 查询和索引

普通列表直接查询 `forum_thread` 当前记录。删除现有列表排序历史、提交水位及对应全社区根锁；正文修订历史继续保留。

| 查询 | 核心索引 |
|---|---|
| 全站最新回复 | `(lastpost DESC, tid DESC)` |
| 版块最新回复 | `(fid, lastpost DESC, tid DESC)` |
| 全站／版块最新主题 | `(dateline DESC, tid DESC)`／`(fid, dateline DESC, tid DESC)` |
| 作者主题 | `(authorid, dateline DESC, tid DESC)` |
| 热门主题 | `(views DESC, tid DESC)` |
| 精华 | `(lastpost DESC, tid DESC)`，限定 `digest>0` |
| 置顶 | `(displayorder DESC, lastpost DESC, tid DESC)`，限定 `displayorder>0` |
| 主题楼层 | 主键 `(tid,position)` |
| 作者回复 | `(authorid,dateline DESC,pid DESC)`，限定非首帖 |
| 只看作者 | `(tid,authorid,position)` |
| 通知全部／未读 | `(uid,dateline DESC,id DESC)`，另建未读部分索引 |
| 标签关系 | 唯一 `(tagid,itemid,idtype)`；反向 `(idtype,itemid,tagid)` |

面向公开列表的主题索引采用固定发布、审核条件；作者本人待审和草稿查询使用覆盖其范围的作者索引。主体权限仍在查询中校验。普通列表允许作者删除后的主题占位，热门按现有规则排除，不能统一套用 `deleted_at IS NULL`。

固定实现规则：

- 每种排序使用独立 SQL，首屏和续页分开；不使用参数化 `CASE` 切换排序。
- 游标携带排序值和唯一 ID，并签名绑定主体、筛选条件及排序方式。
- 置顶独立查询，普通流排除当前展示的置顶主题；全局和版块置顶按范围展示。
- 标签通过关联表查询主题，不复制第二份主题排序事实；稀疏标签和大标签分别验证执行计划。
- 回复按楼层范围读取，默认每页 50 条回复。首次遍历记录最大楼层，新回复留给刷新。
- 作者资料、标签、互动状态批量加载，数据查询数量不得随页面记录数增长。
- 部分索引的固定业务谓词写入相应静态 SQL，查询值仍绑定参数；不依赖一个万能查询自动匹配所有部分索引。

索引按实际筛选与排序验证，不因“存在索引”就认定性能通过；PostgreSQL 的复合索引前导列与排序规则决定其适用范围。参考 [PostgreSQL 16 复合索引](https://www.postgresql.org/docs/16/indexes-multicolumn.html)、[索引与排序](https://www.postgresql.org/docs/16/indexes-ordering.html)、[部分索引](https://www.postgresql.org/docs/16/indexes-partial.html)。

### 3.2 搜索和写事务

- 搜索从零重建，继续采用 `pg_jieba + pg_trgm`；不复制 Discuz 默认的正文 `LIKE` 和搜索结果 ID 字符串缓存。
- 搜索文档、正文版本和可见性与内容同事务更新；过滤当前权限后才能产生摘要或高亮。
- 主题创建、回复、楼层分配、有效计数、修订、幂等结果和 Outbox 同事务提交；适用的审计和业务账本分录同样在该事务完成。
- 楼层通过主题行原子递增并 `RETURNING maxposition` 获取，不使用 `MAX()+1` 或 Redis 发号。
- 锁限定在参与业务的身份、版块、主题等记录，按固定顺序获取；共享版块计数的竞争仍需测试。
- Redis 继续只承载允许丢失的状态。缓存命中仍须检查当前权限，Outbox 负责提交后失效与外部副作用，不补做核心业务状态或权威搜索权限。
- 提供计数重算和一致性校验命令，便于发现与修复冗余数据偏差。

PostgreSQL 单库基线实施时解除三库交集限制，允许使用 `RETURNING`、`ON CONFLICT` 和部分索引等 PostgreSQL 能力；该变化须先在正式架构和相关规则中同步，不表示本次文档归档已经改动现有代码约束。

## 四、基建替换与实施顺序

在现有 `plan/` 内登记重建决策和 `WP-DBR-01` 至 `WP-DBR-05`，不建立第二套计划入口。以下为拟登记工作包，不在本文维护执行状态。

1. **WP-DBR-01：更新事实源。**

   同步产品、架构、后台、路线、质量与追踪文档，以及受影响的本机规则。明确撤销多库支持、15 个领域 Schema、三库 SQL 限制及冻结列表分页；建立“需求 ID → Discuz 来源 → 新表 → 查询 → 索引 → 测试”映射。保留未被本轮明确调整的需求 ID、阶段和验收规则。

2. **WP-DBR-02：新建 PostgreSQL 底座。**

   重写连接、事务、迁移、命名、角色权限、种子和 CLI 数据库命令。移除 SQLite/MySQL 配置、依赖和迁移轨；合并 PostgreSQL 结构及搜索迁移为单轨，从新基线开始。迁移具备锁、校验和、失败回滚及基线识别。

3. **WP-DBR-03：接通核心论坛。**

   按账号与邀请 → 版块 → 主题与回复 → 列表与详情完成纵向切片。同步建立最小审计、幂等和 Outbox，避免后补事务正确性。

4. **WP-DBR-04：接通其余产品能力。**

   重写搜索、互动、通知、管理、治理、积分及其他已实现功能的持久化接线。尚未实现的产品能力保持原阶段，届时按新模型建设。

5. **WP-DBR-05：清理与硬化。**

   删除旧迁移、方言抽象、Repository、数据库夹具和旧命令实现；重新验证新版本。旧测试记录保留历史出处，不用于证明新设计完成。

内部数据库接口统一为 `PgPool` 和借用的 `PgConnection`；应用服务管理事务，Repository 不自行提交，HTTP Handler 不直接写 SQL。保留既有 HTTP 路由、视图契约及 API ID 字符串格式；旧游标失效后提示重新加载列表。

实施使用新的本地开发库，不清空远程数据库，不部署、不推送。受影响需求重新登记进度和证据，功能开关继续受阶段门禁控制。数据库基建及业务接线整体重写，不能仅修改 DDL 后沿用旧 Repository 并宣称完成重建。

## 五、验收与默认边界

开发期将 SQLite 冒烟替换为本地 PostgreSQL 最小冒烟，保留适用单元测试、格式检查、Clippy、Diff 和暂存区复核；该调整随新基线同步质量规范，完整矩阵仍在统一硬化阶段执行。

重点验收：

- 空库初始化、重复迁移、基线不匹配拒绝、默认及非默认表前缀、最小数据库权限。
- 并发注册和邀请码消费、回复楼层唯一、失败整笔回滚、请求重试不重复写入。
- 同时间排序、深游标、实时顶帖、永久楼层、只看作者与删除占位。
- 移帖、审核、恢复、匿名化后的权限、计数、附件及搜索一致性。
- 后台 RBAC＋ABAC、职责分离、权限版本失效、敏感读取审计失败关闭、C3 永不返回。
- 首页、版块、作者、标签、热门、搜索和大帖的 `EXPLAIN (ANALYZE, BUFFERS)`，以及完整 HTTP/SSR 负载测试。

正式性能门槛沿用现有批准值：

| 指标 | 门槛 |
|---|---|
| 注册用户 | 10,000 |
| 主题与回复 | 合计 100,000 |
| 峰值在线 | 200 |
| 公开读负载 | 100 read/s |
| 写负载 | 10 write/s |
| 公开读 p95 | ≤400ms |
| 写 p95 | ≤800ms |
| SSR 首屏 p95 | ≤800ms |
| 删除、隐藏、权限收紧向搜索与缓存的传播 | ≤60 秒，传播期间不得放宽源对象权限 |

参数出处为 [`90-quality-release.md`](90-quality-release.md) 第 6 节及 `DEC-069/070/071`；百万内容作为额外容量观察，不将源码分析或小数据结果外推为达标。

截至本文归档，仅完成方案研究与文档记录，未实施数据库重建，未授予新的数据库测试通过、`Implemented`、`Verified` 或发布状态。
