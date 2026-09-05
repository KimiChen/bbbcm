#!/usr/bin/env python3
"""供应链入口的开发期测试；只扫描临时合成夹具，不扫描完整候选。"""

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("scanner", Path(__file__).with_name("supply-chain-scan.py"))
scanner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scanner)
b = scanner.baseline


class BaselineSafetyTests(unittest.TestCase):
    def test_corrupt_cache_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(b, "CACHE", Path(temp)):
            digest = hashlib.sha256(b"expected").hexdigest()
            (Path(temp) / "downloads").mkdir()
            (Path(temp) / "downloads" / digest).write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "缓存摘要不符"):
                b.artifact({"url": "https://example.invalid/archive", "sha256": digest}, offline=True)

    def test_offline_never_downloads(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(b, "CACHE", Path(temp)), patch.object(b.urllib.request, "urlopen") as get:
            with self.assertRaisesRegex(ValueError, "离线缓存缺失"):
                b.artifact({"url": "https://example.invalid/archive", "sha256": "0" * 64}, offline=True)
            get.assert_not_called()

    def test_archive_path_escape_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "bad.tar"
            with tarfile.open(archive, "w") as tar:
                member = tarfile.TarInfo("../escape")
                member.size = 1
                tar.addfile(member, io.BytesIO(b"x"))
            with self.assertRaises(tarfile.FilterError):
                b.extract(archive, root / "out")
            self.assertFalse((root / "escape").exists())

    def test_unknown_license_stays_unknown(self):
        result = scanner.license_inventory({"components": [{"name": "unconfirmed"}]})
        self.assertEqual(result[0]["status"], "unknown-needs-review")
        self.assertEqual(result[0]["licenses"], [])

    def test_vulnerability_gate_keeps_all_findings_but_blocks_high(self):
        report = {"Results": [{"Vulnerabilities": [{"Severity": s} for s in ["LOW", "UNKNOWN", "HIGH", "CRITICAL"]]}]}
        self.assertEqual(len(scanner.critical_high(report)), 2)
        self.assertEqual(len(report["Results"][0]["Vulnerabilities"]), 4)

    def test_cpp_and_optional_npm_are_included(self):
        bom = {"components": [{"name": "serde", "version": "1.0.0", "purl": "pkg:cargo/serde@1.0.0"}]}
        metadata = {"packages": [{"name": "serde", "version": "1.0.0", "license": "MIT OR Apache-2.0"}]}
        npm = {"components": [{"name": "@fixture/optional", "version": "1.0.0", "purl": "pkg:npm/%40fixture/optional@1.0.0"}]}
        result = scanner.enrich_bom(bom, metadata, b.read_json(b.LOCK_PATH), npm)
        self.assertEqual({c["name"] for c in result["components"]}, {"serde", "@fixture/optional", "pg_jieba", "cppjieba", "limonp"})
        self.assertEqual(result["components"][0]["licenses"], [{"expression": "MIT OR Apache-2.0"}])
        npm_lock = {"packages": {"": {"name": "fixture"}, "node_modules/@fixture/optional": {"version": "1.0.0"}}}
        scanner.check_npm_coverage(npm_lock, npm)
        # 名称数量相同、版本错误也必须失败；不能靠其他组件补足计数。
        npm_lock["packages"]["node_modules/@fixture/optional"]["version"] = "2.0.0"
        with self.assertRaisesRegex(ValueError, "npm SBOM 缺少锁定包或版本"):
            scanner.check_npm_coverage(npm_lock, result)


class ToolContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        for name in ("node", "syft", "trivy", "gitleaks"):
            b.verify_tool(name)

    def test_esbuild_native_transform(self):
        result = b.run([b.tool_path("node"), "-e", "const e=require('esbuild'); const r=e.transformSync('const n: number = 1', {loader:'ts'}); if(r.code.includes(': number')) process.exit(1); console.log(e.version)"], capture=True)
        self.assertEqual(result, b.read_json(b.ROOT / "package.json")["devDependencies"]["esbuild"])

    def test_sbom_and_license_config_on_small_fixture(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            # 合成项目只含一个 Rust crate 和一个 npm 开发依赖。
            (root / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "serde"\nversion = "1.0.228"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n')
            b.write_json(root / "package.json", {"name": "baseline-fixture", "version": "1.0.0", "private": True, "devDependencies": {"esbuild": "0.28.2"}})
            lock = {"name": "baseline-fixture", "version": "1.0.0", "lockfileVersion": 3,
                    "packages": {"": {"name": "baseline-fixture", "version": "1.0.0", "devDependencies": {"esbuild": "0.28.2"}},
                                 "node_modules/esbuild": {"version": "0.28.2", "dev": True, "license": "MIT", "resolved": "https://registry.npmjs.org/esbuild/-/esbuild-0.28.2.tgz"}}}
            b.write_json(root / "package-lock.json", lock)
            report = root / "fixture.cdx.json"
            scanner.command([b.tool_path("syft"), "scan", "dir:" + str(root), "--config", scanner.CONFIG / "syft.yaml", "-o", "cyclonedx-json=" + str(report)])
            bom = b.read_json(report)
            self.assertTrue({"serde", "esbuild"} <= {c["name"] for c in bom["components"]})
            npm_bom = json.loads(b.npm(["sbom", "--package-lock-only", "--sbom-format=cyclonedx", "--include=dev"], cwd=root, capture=True))
            self.assertIn("esbuild", {c["name"] for c in npm_bom["components"]})
            bom = scanner.enrich_bom(bom, {"packages": [{"name": "serde", "version": "1.0.228", "license": "MIT OR Apache-2.0"}]}, b.read_json(b.LOCK_PATH), npm_bom)
            b.write_json(report, bom)
            license_report = root / "licenses.json"
            scanner.command([*scanner.trivy_args("license", license_report), report], license_report)
            results = b.read_json(license_report)
            detected = [entry for result in results.get("Results", []) for entry in result.get("Licenses", [])]
            self.assertTrue(detected, "Trivy 没有返回任何许可证结果")
            self.assertIn("MIT", {entry["Name"] for entry in detected})

    def test_missing_vulnerability_db_fails_offline(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            sbom = root / "fixture.cdx.json"
            b.write_json(sbom, {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1, "components": []})
            with patch.object(b, "CACHE", root / "empty-cache"):
                args = [*scanner.trivy_args("vuln", root / "vulns.json"), sbom]
                result = subprocess.run([str(a) for a in args], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("first run", result.stderr.lower())

    def test_gitleaks_detects_synthetic_secret_and_redacts(self):
        with tempfile.TemporaryDirectory() as temp:
            report = Path(temp) / "secrets.json"
            # 临时生成无效的 GitHub token 格式；只验证识别与脱敏，不访问服务。
            fake = "ghp_" + hashlib.sha256(b"bbbcm-invalid-synthetic-token").hexdigest()[:36]
            args = [b.tool_path("gitleaks"), "stdin", "--config", scanner.CONFIG / "gitleaks.toml",
                    "--redact=100", "--no-banner", "--report-format", "json", "--report-path", report]
            result = subprocess.run([str(a) for a in args], input="token=" + fake + "\n", capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            self.assertTrue(b.read_json(report))
            self.assertNotIn(fake, report.read_text() + result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
