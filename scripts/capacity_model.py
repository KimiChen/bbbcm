#!/usr/bin/env python3
"""WP-M0-05 派生模型校验与有限合成夹具；没有网络、数据库或负载执行器。"""

import argparse
from collections import Counter
from datetime import datetime, timedelta, timezone
import hashlib
import json
import math
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
MODEL_PATH = ROOT / "docs/performance/capacity-baseline-v1.json"
TARGETS = {
    "registered_users": 10000,
    "peak_online_sessions": 300,
    "topics_and_replies": 1000000,
    "offered_public_reads_per_second": 200,
    "offered_writes_per_second": 20,
    "public_read_p95_ms": 400,
    "write_p95_ms": 800,
    "ssr_first_byte_p95_ms": 800,
}


def load_model():
    model = json.loads(MODEL_PATH.read_text(encoding="utf-8"))
    validate_model(model)
    return model


def positive_integer(value):
    return type(value) is int and value > 0


def allocate(total, percentages):
    """最大余数法，平分余数按输入顺序；精确合计而非逐项四舍五入。"""
    if not positive_integer(total) or not percentages:
        raise ValueError("总数和比例不能为空")
    if any(not positive_integer(p) for p in percentages) or sum(percentages) != 100:
        raise ValueError("比例必须为正整数并合计 100")
    counts = [total * p // 100 for p in percentages]
    order = sorted(range(len(percentages)), key=lambda i: (-(total * percentages[i] % 100), i))
    for i in order[:total - sum(counts)]:
        counts[i] += 1
    return counts


def validate_model(model):
    if model.get("model_version") != "capacity-baseline-v1":
        raise ValueError("未知模型版本")
    if model.get("kind") != "derived_model_not_measurement" or model.get("targets") != TARGETS:
        raise ValueError("计划目标漂移或把模型冒充实测")
    a = model["assumptions"]
    for key in ("topics", "replies", "historical_span_days", "topic_title_utf8_bytes",
                "warmup_seconds", "measurement_seconds", "repeat_count", "sample_max_content_rows"):
        if not positive_integer(a[key]):
            raise ValueError(f"{key} 必须为正整数")
    if a["topics"] + a["replies"] != TARGETS["topics_and_replies"]:
        raise ValueError("主题与回复数量没有合计到目标")
    if a["topics"] != 100000 or a["replies"] != 900000:
        raise ValueError("v1 生成算法要求主题/回复为 1:9；修改须升级模型")
    if not isinstance(a["seed"], str) or not a["seed"]:
        raise ValueError("需要固定非空种子")
    for key in ("hot_topic_percent", "hot_reply_percent", "hot_detail_read_percent"):
        if not positive_integer(a[key]) or a[key] >= 100:
            raise ValueError("冷热比例必须在 1..99")
    for key in ("forums", "visibility_cases", "topic_body_utf8_bytes", "reply_body_utf8_bytes",
                "read_scenarios", "write_scenarios"):
        group = a[key]
        allocate(100, [item["percent"] for item in group])
        if key.endswith("utf8_bytes"):
            if any(not positive_integer(item["bytes"]) for item in group):
                raise ValueError("正文单位必须为正 UTF-8 字节")
        elif len({item["id"] for item in group}) != len(group):
            raise ValueError("分布分组 ID 重复")
    if len(a["forums"]) != 4:
        raise ValueError("首发四版块不一致")
    if any(type(item["ssr"]) is not bool for item in a["read_scenarios"]):
        raise ValueError("SSR 标记必须为布尔值")
    if a["arrival_model"] != "open_loop_fixed_rate" or a["sample_max_content_rows"] > 10000:
        raise ValueError("有限样本边界或到达模型漂移")
    if a["synthetic_epoch_utc"] != "2026-01-01T00:00:00.000000Z":
        raise ValueError("v1 合成时间基点漂移")
    if not model.get("pending_production_values") or any(
            value is not None for value in model["pending_production_values"].values()):
        raise ValueError("未批准生产参数必须保持 null")
    measurement = model["measurement"]
    if measurement["executed"] is not False or any(
            value is not None for key, value in measurement.items() if key != "executed"):
        raise ValueError("本模型没有性能实测")


def bucket(index, total, group, field):
    for row, count in zip(group, allocate(total, [row["percent"] for row in group])):
        if index < count:
            return row[field]
        index -= count
    raise ValueError("分布索引越界")


def permuted(index, total, seed, domain):
    """带域的仿射双射，分布不碰撞、不同属性不共用同一排序。"""
    digest = hashlib.sha256(f"{seed}:{domain}".encode()).digest()
    factor = int.from_bytes(digest[:8], "big") % total or 1
    while math.gcd(factor, total) != 1:
        factor += 1
    offset = int.from_bytes(digest[8:16], "big") % total
    return (factor * index + offset) % total


def body_bytes(size, seed, identity):
    """仅合成词汇和确定性摘要；按 UTF-8 字节截断且不截断码点。"""
    chunks = []
    total = 0
    counter = 0
    while total < size:
        digest = hashlib.sha256(f"{seed}:{identity}:{counter}".encode()).hexdigest()
        part = f"草莓论坛 合成内容 数据库 Rust AI 域名 云服务 {digest}\n"
        chunks.append(part)
        total += len(part.encode("utf-8"))
        counter += 1
    text = "".join(chunks).encode("utf-8")[:size].decode("utf-8", errors="ignore")
    return text + "x" * (size - len(text.encode("utf-8")))


def dataset_rows(model, content_rows):
    """小样本逻辑行，禁止生成百万级实体；不创建业务账号或物理表。"""
    validate_model(model)
    a = model["assumptions"]
    if (not positive_integer(content_rows) or content_rows < 100 or content_rows % 100
            or content_rows > a["sample_max_content_rows"]):
        raise ValueError("content_rows 必须为 100 的倍数且位于 100..10000，禁止本轮百万灌数")
    user_count = content_rows // 100
    topic_count, reply_count = content_rows // 10, content_rows * 9 // 10
    seed = a["seed"]
    epoch = datetime(2026, 1, 1, tzinfo=timezone.utc)
    start = epoch - timedelta(days=a["historical_span_days"])

    def timestamp(value):
        return value.isoformat(timespec="microseconds").replace("+00:00", "Z")

    yield {"kind": "manifest", "model_version": model["model_version"], "seed": seed,
           "synthetic": True, "sample_only": True, "registered_users": user_count,
           "topics": topic_count, "replies": reply_count, "full_target_content": 1000000}
    for i in range(user_count):
        yield {"kind": "user_fixture", "ref": f"user-{i + 1}", "created_at": timestamp(start),
               "display_name": f"合成果农{i + 1}", "login_capability": False}

    def topic_metadata(i):
        rank = lambda domain: permuted(i, topic_count, seed, domain)
        created = start + timedelta(seconds=rank("age") * a["historical_span_days"] * 86400 // topic_count)
        return {"forum_ref": bucket(rank("forum"), topic_count, a["forums"], "id"),
                "visibility_case": bucket(rank("visibility"), topic_count, a["visibility_cases"], "id"),
                "created_at": timestamp(created)}

    for i in range(topic_count):
        size = bucket(permuted(i, topic_count, seed, "topic-body"), topic_count,
                      a["topic_body_utf8_bytes"], "bytes")
        yield {"kind": "topic_fixture", "ref": f"topic-{i + 1}",
               "author_ref": f"user-{permuted(i, topic_count, seed, 'topic-author') % user_count + 1}",
               **topic_metadata(i), "version": 1,
               "hot": i < allocate(topic_count, [a["hot_topic_percent"], 100 - a["hot_topic_percent"]])[0],
               "title": body_bytes(a["topic_title_utf8_bytes"], seed, f"title-{i}"),
               "body": body_bytes(size, seed, f"topic-{i}"), "body_utf8_bytes": size}

    hot_topics = allocate(topic_count, [a["hot_topic_percent"], 100 - a["hot_topic_percent"]])[0]
    hot_replies = allocate(reply_count, [a["hot_reply_percent"], 100 - a["hot_reply_percent"]])[0]
    floors = Counter()
    for i in range(reply_count):
        if i < hot_replies:
            parent = i % hot_topics
        else:
            parent = hot_topics + (i - hot_replies) % (topic_count - hot_topics)
        floors[parent] += 1
        size = bucket(permuted(i, reply_count, seed, "reply-body"), reply_count,
                      a["reply_body_utf8_bytes"], "bytes")
        meta = topic_metadata(parent)
        created = datetime.fromisoformat(meta["created_at"].replace("Z", "+00:00"))
        # 回复落在主题与固定 epoch 之间；楼层递增不会产生时间倒退。
        fanout = (hot_replies + hot_topics - 1) // hot_topics if parent < hot_topics else (
            reply_count - hot_replies + topic_count - hot_topics - 1) // (topic_count - hot_topics)
        remaining_us = (epoch - created) // timedelta(microseconds=1)
        created += timedelta(microseconds=remaining_us * floors[parent] // (fanout + 1))
        yield {"kind": "reply_fixture", "ref": f"reply-{i + 1}", "topic_ref": f"topic-{parent + 1}",
               "author_ref": f"user-{permuted(i, reply_count, seed, 'reply-author') % user_count + 1}",
               "floor_no": floors[parent] + 1, "version": 1, **meta,
               "created_at": timestamp(created), "body": body_bytes(size, seed, f"reply-{i}"),
               "body_utf8_bytes": size}


def nearest_rank(values, percentile=95):
    if not values:
        return None
    if not 0 < percentile <= 100 or any(not math.isfinite(v) or v < 0 for v in values):
        raise ValueError("分位数或时长非法")
    return sorted(values)[math.ceil(percentile * len(values) / 100) - 1]


def summarize_samples(samples):
    """仅校验测量算法；生产报告必须另附窗口、分层和调度遗漏计数。"""
    allowed = {"success", "error", "timeout", "not_started"}
    outcomes = Counter()
    observed, successes = [], []
    for sample in samples:
        outcome = sample["outcome"]
        value = sample.get("latency_ms")
        if outcome not in allowed:
            raise ValueError("未知测量结果")
        outcomes[outcome] += 1
        if outcome in {"success", "error"}:
            if type(value) not in {int, float} or not math.isfinite(value) or value < 0:
                raise ValueError("已完成请求必须有有限非负时长")
            observed.append(value)
            if outcome == "success":
                successes.append(value)
        elif value is not None:
            raise ValueError("超时/未发出不能伪装成完成时长；下界另列")
    n = sum(outcomes.values())
    rank = math.ceil(0.95 * n) if n else 0
    conservative = sorted(successes)[rank - 1] if rank and rank <= len(successes) else None
    return {"offered": n, "outcomes": dict(sorted(outcomes.items())),
            "observed_response_p95_ms": nearest_rank(observed),
            "success_latency_p95_ms": nearest_rank(successes),
            "conservative_offered_p95_ms": conservative,
            "conservative_offered_p95_unbounded": bool(n and conservative is None),
            "performance_pass": None}


def model_summary(model):
    validate_model(model)
    a, targets = model["assumptions"], model["targets"]
    logical_body = sum(count * row["bytes"] for total, key in (
        (a["topics"], "topic_body_utf8_bytes"), (a["replies"], "reply_body_utf8_bytes"))
        for count, row in zip(allocate(total, [row["percent"] for row in a[key]]), a[key]))
    return {"kind": "model_arithmetic_not_measurement", "model_version": model["model_version"],
            "target_content_rows": a["topics"] + a["replies"],
            "logical_body_bytes": logical_body, "logical_body_mib": logical_body / 1024 ** 2,
            "logical_title_bytes": a["topics"] * a["topic_title_utf8_bytes"],
            "offered_reads_per_window": targets["offered_public_reads_per_second"] * a["measurement_seconds"],
            "offered_writes_per_window": targets["offered_writes_per_second"] * a["measurement_seconds"],
            "read_rates_per_second": {row["id"]: targets["offered_public_reads_per_second"] * row["percent"] / 100
                                      for row in a["read_scenarios"]},
            "write_rates_per_second": {row["id"]: targets["offered_writes_per_second"] * row["percent"] / 100
                                       for row in a["write_scenarios"]},
            "performance_measured": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "sample"))
    parser.add_argument("--content-rows", type=int, default=1000)
    args = parser.parse_args()
    model = load_model()
    if args.command == "check":
        print(json.dumps(model_summary(model), ensure_ascii=False, indent=2))
    else:
        for row in dataset_rows(model, args.content_rows):
            print(json.dumps(row, ensure_ascii=False, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError) as error:
        print(f"容量模型校验失败：{error}", file=sys.stderr)
        raise SystemExit(1) from error
