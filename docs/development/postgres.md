# PostgreSQL 集成合同运行手册（E01）

本手册是 [`90` 第 5 节](../../plan/90-quality-release.md)「PostgreSQL 集成」的派生操作说明，范围与状态见 [`99` 第 5.23 节](../../plan/99-traceability.md)。载体是 `crates/platform-db/tests/postgres_contract.rs`，默认 `#[ignore]`，开发期最小集不会执行它。

## 目标库必须是一次性的

测试会建 schema、建表、写数据。入口处有两道闸门：`POSTGRES_CONTRACT_ISOLATED=yes` 的显式确认，以及连接串必须是**字面回环 IP**。

这两道闸门只证明「连的是本机回环」，**不证明库是空的、可丢弃的**。后者由运行者保证：用一次性容器或一次性本地实例，跑完销毁。`AGENTS.md` 明令「不得以开发期为由擅自清空非本地数据库」，指错一次 URL 就足以把集成测试变成一次生产事故。

任何能提供隔离 PostgreSQL 16 的方式都可以：本机安装、本机容器、或到一次性远端实例的 SSH 本地端口转发。手册不记录具体主机。

## 一、起一个隔离实例

镜像取 `supply-chain/lock.json` 的 `images.postgres`，按 **index 摘要**拉取而不是按 tag，这样拉到的就是锁定的那一个：

```bash
docker pull "docker.io/library/postgres@$(python3 -c "import json;print(json.load(open('supply-chain/lock.json'))['images']['postgres']['index_digest'])")"
```

启动时必须给 UTF-8 locale。`prepare_postgres` 会拒绝 `C`/`POSIX` 的 `datctype`，而官方镜像默认就是 `C`：

```bash
docker run -d --name bbbcm-e01-pg \
  -p 127.0.0.1:5433:5432 \
  -e POSTGRES_HOST_AUTH_METHOD=trust \
  -e POSTGRES_DB=bbbcm_contract \
  -e POSTGRES_INITDB_ARGS="--encoding=UTF8 --locale=C.UTF-8" \
  --tmpfs /var/lib/postgresql/data:rw,size=4g \
  "docker.io/library/postgres@sha256:<index 摘要>" \
  -c max_connections=200 -c fsync=off -c synchronous_commit=off
```

用 `trust` 是刻意的：一次性容器只绑回环，不设口令就没有凭据可泄露、可误提交。`--tmpfs` 让数据随容器一起消失，`fsync=off` 只对一次性实例成立，不代表生产配置。

## 二、准备角色

`prepare_postgres` 对角色的要求是**硬前置**，不是建议：

- 四个用途组（`<prefix>migrator`/`app_rw`/`worker_rw`/`report_ro`）必须 `NOLOGIN NOINHERIT`，且自身不是任何角色的成员。
- 运行登录角色的组成员关系必须**恰好一条**——自己的用途组，`INHERIT FALSE`、`SET TRUE`。
- 运行登录角色不得拥有该数据库、不得有 `CREATE`/`TEMPORARY`、不得在任何 schema 上有 `CREATE`、不得对业务对象有直接授权。

**因此迁移与运行必须是两个登录角色**：同一个角色不可能既是 `migrator` 的成员又只有一条成员关系。这是设计要求，不是测试图方便。

组角色由 CLI 生成的 SQL 建立（`\gset` 是 psql 元命令，必须用 psql 执行，不能走 `sqlx`）：

```bash
BBBCM_DATABASE_BACKEND=postgres \
BBBCM_WORKER_EMBEDDED=false \
BBBCM_DATABASE_POSTGRES_URL="postgres://postgres@127.0.0.1:5433/bbbcm_contract" \
BBBCM_DATABASE_SCHEMA_PREFIX="$PREFIX" \
cargo run -p bbbcm-cli -- db bootstrap-sql
```

把输出交给 psql，再补两个登录角色（`NOINHERIT` 不继承组的 `CONNECT`，所以要直接授予）：

```sql
CREATE ROLE "<prefix>mig_login" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT CONNECT ON DATABASE bbbcm_contract TO "<prefix>mig_login";
GRANT "<prefix>migrator" TO "<prefix>mig_login" WITH INHERIT FALSE, SET TRUE;

CREATE ROLE "<prefix>app_login" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT CONNECT ON DATABASE bbbcm_contract TO "<prefix>app_login";
GRANT "<prefix>app_rw" TO "<prefix>app_login" WITH INHERIT FALSE, SET TRUE;
```

角色名带 schema 前缀，所以**默认前缀和随机非默认前缀各需要一整套**。

## 三、随机非默认前缀每次运行都要是新的

`migrations_apply_once_then_skip_and_two_prefixes_stay_isolated` 断言随机前缀上「全部应用、零跳过」。这条断言正是它的价值所在，所以每次运行都要生成一个**没被迁移过**的新前缀，并为它建好上面两个登录角色。复用上一轮的前缀会让这条测试失败——那是断言在正确工作，不是测试有问题。

## 四、运行

```bash
POSTGRES_CONTRACT_ISOLATED=yes \
POSTGRES_CONTRACT_URL='postgres://{prefix}app_login@127.0.0.1:5433/bbbcm_contract' \
POSTGRES_CONTRACT_MIGRATOR_URL='postgres://{prefix}mig_login@127.0.0.1:5433/bbbcm_contract' \
POSTGRES_CONTRACT_ALT_PREFIX="$FRESH_PREFIX" \
cargo test -p platform-db --test postgres_contract -- --ignored
```

字面量 `{prefix}` 由测试按当前 schema 前缀替换。连接串从不打印到测试输出——它可能带凭据，而测试输出会进证据包。

## 覆盖范围

| 测试 | `90` §5 对应项 |
|---|---|
| `migrations_apply_once_then_skip_and_two_prefixes_stay_isolated` | 空库迁移、重复运行幂等、编号连续、默认与随机非默认前缀并存且互不可见 |
| `trusted_identifiers_are_whitelisted_and_values_are_always_bound` | 受信标识符渲染（敌意前缀、未登记表在渲染器层被拒）、bind 参数逐字节往返 |
| `app_role_cannot_write_reference_tables_but_can_read_them` | ACL 最小权限：运行角色对 `community.forums` 只读 |
| `database_constraints_reject_invalid_rows` | 唯一、CHECK、外键由数据库兜底 |
| `rollback_leaves_no_trace` | 事务回滚，且换连接回读确认是数据库级别 |
| `version_conflict_prevents_lost_update` | 版本防覆盖，冲突分支无部分生效 |
| `idempotency_replays_same_payload_and_rejects_different_payload` | 同键同载荷重放返回原结果、同键异载荷冲突、行数不增 |
| `outbox_event_and_business_fact_commit_together_or_not_at_all` | Outbox 事件与业务事实同事务提交：提交后都在、回滚后都不在 |
| `future_objects_grant_nothing_to_runtime_roles_by_default` | 未来对象默认权限失败关闭；函数必须留下显式条目以撤销 PUBLIC 的 EXECUTE |
| `lock_order_is_frozen_and_append_phase_forbids_further_locks` | `DEC-052` 固定锁顺序：升序放行、乱序拒绝、追加段内不得补锁 |
| `concurrent_same_key_commands_produce_exactly_one_row` | 8 路并发同幂等键：恰好一路首写，业务事实只有一条 |

## 尚未覆盖

以下 `90` §5 条目**本载体不含**，不得据本手册宣称已验证：

- **未知提交（`CommitUnknown`）**：触发它需要 `command.rs` 里 `#[cfg(test)]` 的 `simulate_lost_ack` 钩子，集成测试在 crate 外拿不到，只能由 crate 内单元测试覆盖。
- **`40001`/`40P01` 的受控重试**：要压出序列化失败或死锁，必须**故意违反**固定锁顺序，而平台层会先一步拒绝（见 `lock_order_is_frozen_…`）。这条要么另建绕过平台层的夹具，要么承认它被设计挡在了发生之前。
- **审计与搜索权限状态的同事务提交**：本载体只验证了 Outbox 一侧。
- **`pg_jieba`/`pg_trgm` 完整搜索模式**：需按 `M0-03` 的 Dockerfile 构建扩展镜像。

## 五、销毁

```bash
docker rm -f bbbcm-e01-pg
```

`--tmpfs` 数据目录随容器消失。若用的是本机常驻实例，需自行删除本轮创建的 schema 与角色。
