#!/usr/bin/env python3
"""WP-M0-05 小夹具与计算口径测试，不导入数据库、不压测。"""

from collections import Counter, defaultdict
import copy
from datetime import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import unittest

import capacity_model as capacity


class CapacityModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.model = capacity.load_model()
        cls.rows = list(capacity.dataset_rows(cls.model, 1000))

    def test_targets_match_original_plan_clauses(self):
        source = (capacity.ROOT / "plan/01-product-requirements.md").read_text(encoding="utf-8")
        self.assertIn("1 万注册用户、300 峰值在线、约 100 万主题与回复", source)
        self.assertIn("200 read/s、20 write/s", source)
        for clause in ("常用公开只读接口 p95 不超过 400ms", "写接口 p95 不超过 800ms",
                       "首屏服务端响应 p95 不超过 800ms"):
            self.assertIn(clause, source)
        self.assertEqual(self.model["targets"], capacity.TARGETS)

    def test_model_arithmetic_and_utf8_unit(self):
        result = capacity.model_summary(self.model)
        self.assertEqual(result["target_content_rows"], 1000000)
        self.assertEqual(result["logical_body_bytes"], 531200000)
        self.assertEqual(result["logical_title_bytes"], 9600000)
        self.assertEqual(result["logical_body_mib"], 531200000 / 1048576)
        self.assertEqual(result["offered_reads_per_window"], 180000)
        self.assertEqual(result["offered_writes_per_window"], 18000)
        self.assertEqual(sum(result["read_rates_per_second"].values()), 200)
        self.assertEqual(sum(result["write_rates_per_second"].values()), 20)
        self.assertIs(result["performance_measured"], False)

    def test_missing_or_unapproved_values_are_not_measurements(self):
        mutations = [
            lambda m: m["targets"].update(public_read_p95_ms=401),
            lambda m: m["assumptions"].update(replies=899999),
            lambda m: m["assumptions"].update(measurement_seconds=True),
            lambda m: m["assumptions"]["read_scenarios"][0].update(percent=46),
            lambda m: m["pending_production_values"].update(vps_cpu_ram="guessed"),
            lambda m: m["measurement"].update(executed=True),
            lambda m: m["measurement"].update(performance_results={"p95": 1}),
            lambda m: m["assumptions"].update(sample_max_content_rows=1000000),
        ]
        for mutation in mutations:
            model = copy.deepcopy(self.model)
            mutation(model)
            with self.assertRaises(ValueError):
                capacity.validate_model(model)

    def test_counts_rounding_and_full_generation_guard(self):
        self.assertEqual(capacity.allocate(101, [70, 25, 5]), [71, 25, 5])
        for total in (1, 3, 11, 99, 1000000):
            self.assertEqual(sum(capacity.allocate(total, [90, 8, 2])), total)
        for invalid in (0, 1, 99, 101, 10001, 1000000, True):
            with self.assertRaises(ValueError):
                list(capacity.dataset_rows(self.model, invalid))
        counts = Counter(row["kind"] for row in self.rows)
        self.assertEqual(counts, {"manifest": 1, "user_fixture": 10,
                                  "topic_fixture": 100, "reply_fixture": 900})
        self.assertFalse(any(row.get("login_capability", False) for row in self.rows))

    def test_seed_reproduces_exact_bytes_and_changes_content(self):
        def digest(rows):
            data = "\n".join(json.dumps(row, ensure_ascii=False, sort_keys=True) for row in rows)
            return hashlib.sha256(data.encode("utf-8")).hexdigest()
        self.assertEqual(digest(self.rows), digest(capacity.dataset_rows(self.model, 1000)))
        model = copy.deepcopy(self.model)
        model["assumptions"]["seed"] = "another-synthetic-seed"
        self.assertNotEqual(digest(self.rows), digest(capacity.dataset_rows(model, 1000)))
        # 固定域双射不重复、不遗漏，防止用取模随机数破坏精确分布。
        self.assertEqual({capacity.permuted(i, 1000, "seed", "domain") for i in range(1000)}, set(range(1000)))

    def test_relations_time_order_and_unique_floors(self):
        users = {row["ref"] for row in self.rows if row["kind"] == "user_fixture"}
        topics = {row["ref"]: row for row in self.rows if row["kind"] == "topic_fixture"}
        floors, times = defaultdict(list), defaultdict(list)
        for row in self.rows:
            if row["kind"] not in {"topic_fixture", "reply_fixture"}:
                continue
            self.assertIn(row["author_ref"], users)
            self.assertEqual(len(row["created_at"]), 27)
            datetime.fromisoformat(row["created_at"].replace("Z", "+00:00"))
            if row["kind"] == "reply_fixture":
                topic = topics[row["topic_ref"]]
                self.assertEqual(row["visibility_case"], topic["visibility_case"])
                self.assertEqual(row["forum_ref"], topic["forum_ref"])
                self.assertGreaterEqual(row["created_at"], topic["created_at"])
                self.assertLessEqual(row["created_at"], self.model["assumptions"]["synthetic_epoch_utc"])
                floors[row["topic_ref"]].append(row["floor_no"])
                times[row["topic_ref"]].append(row["created_at"])
        for topic, values in floors.items():
            self.assertEqual(values, list(range(2, len(values) + 2)))
            self.assertEqual(times[topic], sorted(times[topic]))

    def test_exact_distributions_and_hot_fanout(self):
        topics = [row for row in self.rows if row["kind"] == "topic_fixture"]
        replies = [row for row in self.rows if row["kind"] == "reply_fixture"]
        self.assertEqual(Counter(row["forum_ref"] for row in topics),
                         {"forum-1": 40, "forum-2": 25, "forum-3": 25, "forum-4": 10})
        self.assertEqual(Counter(row["visibility_case"] for row in topics),
                         {"public": 90, "restricted_fixture": 8, "hidden_fixture": 2})
        self.assertEqual(Counter(row["body_utf8_bytes"] for row in topics), {256: 70, 2048: 25, 16384: 5})
        self.assertEqual(Counter(row["body_utf8_bytes"] for row in replies), {128: 630, 512: 225, 4096: 45})
        hot = {row["ref"] for row in topics if row["hot"]}
        self.assertEqual(len(hot), 10)
        self.assertEqual(sum(row["topic_ref"] in hot for row in replies), 540)
        for row in topics + replies:
            self.assertEqual(len(row["body"].encode("utf-8")), row["body_utf8_bytes"])
        self.assertTrue(all(len(row["title"].encode("utf-8")) == 96 for row in topics))

    def test_nearest_rank_uses_samples_not_mean_of_percentiles(self):
        self.assertIsNone(capacity.nearest_rank([]))
        self.assertEqual(capacity.nearest_rank(list(range(1, 101))), 95)
        self.assertEqual(capacity.nearest_rank([800, 1, 2]), 800)
        for values in ([float("nan")], [float("inf")], [-1]):
            with self.assertRaises(ValueError):
                capacity.nearest_rank(values)

    def test_fast_errors_and_missing_samples_cannot_look_like_success(self):
        samples = [{"outcome": "success", "latency_ms": 300} for _ in range(90)]
        samples += [{"outcome": "error", "latency_ms": 1} for _ in range(5)]
        samples += [{"outcome": "timeout"} for _ in range(3)]
        samples += [{"outcome": "not_started"} for _ in range(2)]
        result = capacity.summarize_samples(samples)
        self.assertEqual(result["offered"], 100)
        self.assertEqual(result["observed_response_p95_ms"], 300)
        self.assertEqual(result["success_latency_p95_ms"], 300)
        self.assertIsNone(result["conservative_offered_p95_ms"])
        self.assertTrue(result["conservative_offered_p95_unbounded"])
        self.assertIsNone(result["performance_pass"])
        with self.assertRaises(ValueError):
            capacity.summarize_samples([{"outcome": "timeout", "latency_ms": 1000}])
        with self.assertRaises(ValueError):
            capacity.summarize_samples([{"outcome": "success"}])

    def test_cli_checks_model_and_rejects_million_output(self):
        command = [sys.executable, str(Path(capacity.__file__))]
        result = subprocess.run(command + ["check"], capture_output=True, text=True, check=True)
        self.assertIs(json.loads(result.stdout)["performance_measured"], False)
        result = subprocess.run(command + ["sample", "--content-rows", "1000000"],
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("禁止本轮百万灌数", result.stderr)


if __name__ == "__main__":
    unittest.main()
