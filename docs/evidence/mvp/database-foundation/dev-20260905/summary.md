# 数据库基础开发期证据（2026-09-05 UTC）

入口与状态事实源：[99 §5.4](../../../../../plan/99-traceability.md)。实现顺序 M0-04 → A02 配置前置 → A03。负责人和最终批准人为用户，Codex 负责实施、独立复核、开发期验证和本机提交。代码/配置版本为本文件所在的 `新增：完成数据库基础设计与配置迁移底座` 提交。

## 实际交付

| 工作包 | 开发期状态 | 交付与边界 |
|---|---|---|
| WP-M0-04 | Implemented | [派生设计](../../../../design-notes/2026-09-database-foundation.md)：15 schema、3初始表、受信注册、PG数据库角色/ACL、迁移元数据与恢复、锁总序、幂等、Outbox、追加审计、16类保留矩阵；独立复核完成。事务/Outbox/审计引擎归A04，未以空接口代替实现。 |
| WP-MVP-A02 | In progress（本轮部分完成） | 共用分层配置、4环境模板、local覆盖、Secret注入、严格键/类型/连接/前缀/Redis/Worker校验及3程序接线。剩余Flag持久化、受控写入、A04审计/事务、D04审批、只读/紧急关闭及提示态。 |
| WP-MVP-A03 | Implemented | 两轨真实迁移、编号/内容/历史校验、专用连接/前缀锁、失败/未知提交处理、重建/FK恢复；PG bootstrap/显式和默认ACL、3张必要表、CLI migrate/status/seed、四版块/规则占位、共享SQL lint、真实SQLite dev-check。mysql8仅占位。 |

[操作手册](../../../../development/database.md) 覆盖配置、角色初始化、迁移/status/seed、错误恢复和本机清理。未创建账号、SiteOwner、IAM registry、内容服务或后台，14个现有MVP Flag继续默认关闭。DEC-016/018、CR-013、法律/保留/provider待决未被批准，也未扩大为普通数据库基础阻塞。

## 运行结果

[validation.json](validation.json) 汇总；[dev-check.txt](dev-check.txt) 为完整最小集唯一一次运行输出，主目录绝对路径替换为公开占位，退出码0，无失败或跳过的测试。

- rustfmt、全Workspace/全部target Clippy `-D warnings` 通过。
- 99个单元测试 + 4个真实CLI集成测试全部通过。platform 55、platform-db 29、web 15、database_cli 4。
- 本轮关键覆盖：配置13、迁移20、连接选项4、种子2、数据库ACL1、标识符3、SQL lint8；覆盖默认/非默认前缀、未知表拒绝、内容漂移、元数据损坏/防篡改、空轨、重复执行、真实重建保留既有行、失败回滚、同连接FK恢复、只读status不改journal模式、两生产类环境seed拒绝。
- CLI实际子进程覆盖migrate/status/seed、重复种子、未初始化拒绝、local覆盖、Secret入文件拒绝、mysql8/flag set拒绝以及13个PG环境变量旁路拒绝。测试使用合成Secret哨兵，不使用真实凭证。
- dev-check实际在临时目录迁移SQLite，并通过51条HTTP冒烟；Web 15项既有测试包括模板/资产允许列表和staging/production隔离。HTTP冒烟未连接真实PG。
- [asset-freeze.json](asset-freeze.json) 由 `python3 scripts/asset_freeze.py` 生成，原冻结清单保持，源资产/模板未改；未采集页面、截图或品牌资产。
- `git diff --check`、最终暂存区检查及逐文件复核记录在 [staged-review.json](staged-review.json)。dev-check运行时暂存区为空；最终复核单独执行，不用该空步骤代替审查。

## 设计与代码复核

实现代理细化M0设计并完成迁移，独立配置代理复核共用加载和连接边界，SQL代理复核SQL lint、种子、两轨DDL、ACL与标识符；主代理整合并逐文件核对差异。复核发现和修正了迁移嵌套注释/PG转义文本导致的事务边界绕过、迁移源改写历史、SQLite status隐式切WAL、PG环境/.pgpass隐式配置及runtime登录权限旁路。最终有限复核无未解决阻塞；手册补齐runtime LOGIN自身NOINHERIT及唯一SET/INHERIT/ADMIN成员选项。

开发中针对性检查曾失败：首次锁文件过期阻止编译；迁移异步Result类型推导、pool回调raw_sql生命周期及两项Clippy诊断；SQL lint最初误拒绝受信模板。均已定位修正，没有降级错误处理或放宽Clippy。以上早期结果来自本轮工具/代理报告，未保存完整终端日志，不冒称具有原始运行附件；最终完整检查日志保留于本目录。依赖整理曾产生可选MySQL锁项，已撤回并用锁定PG驱动chrono特性解决，最终未新增依赖包或升级版本，不接入MySQL。

## 未执行与后续依赖

真实PostgreSQL/Redis全矩阵、实际角色成员/未来对象ACL/TLS/扩展拒绝探针、PG并发事务锁、提交未知网络故障注入、安全全量扫描、跨平台、性能和正式视觉验证未执行，留E01/E02/E04。SQL lint是强制静态规则，不替代类型、NULL/排序语义和双后端运行复核。完整资产构建与独立构建未执行（DEC-039，本轮无资产修改）。

A02仍有上表后续交付；M0-04/A03本轮开发交付无剩余缺口。M0其他工作包、Production Approved中的法律、人员、provider、IAM和视觉批准仍未完成。产品需求状态不因骨架/设计推进，未标Verified、未通过阶段/发布门禁。

开始时Git工作区干净。沿用Workspace/工具链；全部既有依赖版本保持。AGENTS.md、g.md、本机配置/Secrets、数据库/WAL/SHM和缓存均不提交；只新建本机中文提交，不amend、不重写、不push、不部署。
