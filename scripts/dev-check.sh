#!/usr/bin/env bash
# 开发期最小集（DEC-020 / plan/90 第 4 节）。任一步失败即中止。
# 不含独立构建步（DEC-039）：Clippy 与测试本就要编译整个工作区。
# 用法：./scripts/dev-check.sh
# 必须能在干净检出上运行，不依赖本机状态。
set -euo pipefail

cd "$(dirname "$0")/.."
fail() { echo "✗ $1" >&2; exit 1; }
step() { echo; echo "== $1"; }

step "1/7 格式化检查"
cargo fmt --all -- --check || fail "格式化不一致，运行 cargo fmt --all"

step "2/7 Clippy -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings || fail "Clippy 有告警"

step "3/7 单元测试"
cargo test --workspace --locked || fail "单元测试失败"

step "4/7 SQLite 冒烟"
if [ -f scripts/smoke-routes.txt ]; then
  SMOKE_DIR="$(mktemp -d)"
  trap 'rm -rf "$SMOKE_DIR"' EXIT
  cat > "$SMOKE_DIR/config.toml" <<CFG
[server]
environment = "test"
bind = "127.0.0.1:18099"
[database]
backend = "sqlite"
schema_prefix = "bcm_"
sqlite_path = "$SMOKE_DIR/smoke.db"
max_connections = 1
[features]
redis = false
[worker]
embedded = true
CFG
  # 迁移执行器随 WP-MVP-A03 接入；在此之前只冒烟 HTTP 表层。
  # cargo test/clippy 不保证刷新 target/debug 下的普通可执行文件。
  # 在冒烟步骤内用 cargo run 运行当前源码，不新增独立构建步骤（DEC-039）。
  if BBBCM_CONFIG="$SMOKE_DIR/config.toml" cargo run --quiet --locked --package bbbcm-cli -- \
      db migrate --backend sqlite >"$SMOKE_DIR/migrate.log" 2>&1; then
    echo "  迁移已执行"
  elif grep -Fq '尚未实现：db migrate --backend sqlite，随 WP-MVP-A03 接入' "$SMOKE_DIR/migrate.log"; then
    echo "  迁移执行器尚未接入，跳过（WP-MVP-A03）"
  else
    cat "$SMOKE_DIR/migrate.log" >&2
    fail "SQLite 迁移冒烟失败"
  fi
  if curl -s --max-time 1 -o /dev/null "http://127.0.0.1:18099/health"; then
    fail "冒烟端口 18099 已有 HTTP 服务，不能复用未知进程"
  fi
  BBBCM_CONFIG="$SMOKE_DIR/config.toml" cargo run --quiet --locked --package bbbcm-web -- \
    >"$SMOKE_DIR/web.log" 2>&1 &
  WEB_PID=$!
  trap 'kill $WEB_PID 2>/dev/null || true; rm -rf "$SMOKE_DIR"' EXIT
  ready=false
  # 干净检出首次运行可能需要链接普通可执行文件，给就绪探测最多两分钟。
  for _ in $(seq 1 600); do
    if ! kill -0 "$WEB_PID" 2>/dev/null; then
      cat "$SMOKE_DIR/web.log" >&2
      fail "HTTP 冒烟进程在就绪前退出"
    fi
    if curl -sf --max-time 1 -o /dev/null "http://127.0.0.1:18099/health"; then
      ready=true
      break
    fi
    sleep 0.2
  done
  if [ "$ready" != true ]; then
    cat "$SMOKE_DIR/web.log" >&2
    fail "HTTP 冒烟服务未在期限内就绪"
  fi
  while read -r route want; do
    [ -z "${route:-}" ] && continue
    case "$route" in \#*) continue ;; esac
    got=$(curl -s --max-time 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:18099$route")
    [ "$got" = "$want" ] || fail "冒烟 $route 期望 $want 实得 $got"
    echo "  $route → $got"
  done < scripts/smoke-routes.txt
  kill $WEB_PID 2>/dev/null || true
else
  echo "  scripts/smoke-routes.txt 不存在，跳过"
fi

step "5/7 git diff --check"
git diff --check || fail "存在行尾空白或冲突标记"

step "6/7 暂存区复核"
git diff --cached --name-only | sed 's/^/  /'
if git diff --cached --name-only | grep -q '^plan/06-' \
   && git diff --cached --name-only | grep -qE '^(crates|apps)/'; then
  echo "  ⚠ 决策变更与实现混在同一次暂存（plan/06 第 5 节要求单独提交）" >&2
fi

step "7/7 静态守卫"
if git grep -n 'sqlx' -- 'crates/*/src/http.rs' 'apps/web/src' | grep -v '^$'; then
  fail "handler 不得直接使用 sqlx（plan/02 第 2.1 节）"
fi
echo "  handler 无 sqlx 引用"

echo
echo "✓ 开发期最小集全部通过"
