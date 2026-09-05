#!/usr/bin/env python3
"""按需生成供应链报告；全量运行归 WP-MVP-E01，不由 dev-check 调用。"""

import argparse
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile

spec = importlib.util.spec_from_file_location("baseline", Path(__file__).with_name("supply-chain.py"))
baseline = importlib.util.module_from_spec(spec)
spec.loader.exec_module(baseline)
ROOT = baseline.ROOT
CONFIG = ROOT / "supply-chain"


def command(args, report=None, *, input_text=None):
    result = subprocess.run([str(a) for a in args], cwd=ROOT, input=input_text, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if report is not None:
        report.with_suffix(".log").write_text(result.stderr, encoding="utf-8")
    baseline.require(result.returncode == 0, f"{Path(args[0]).name} 执行失败（退出码 {result.returncode}），检查本地报告")
    return result.stdout


def trivy_args(scanner, report, mode="sbom"):
    return [baseline.tool_path("trivy"), mode, "--config", CONFIG / "trivy.yaml",
            "--cache-dir", baseline.CACHE / "trivy", "--scanners", scanner,
            "--disable-telemetry", "--skip-version-check", "--offline-scan",
            "--skip-db-update", "--skip-java-db-update", "--ignorefile", CONFIG / "trivy.ignore",
            "--format", "json", "--exit-code", "0", "--output", report]


def enrich_bom(bom, cargo_metadata, lock, npm_bom):
    """Cargo 许可证来自同一锁定源码元数据，C++ 源码明确补入，不靠镜像猜包。"""
    licenses = {(p["name"], p["version"]): p.get("license") for p in cargo_metadata["packages"]}
    components = bom.setdefault("components", [])
    purls = {c.get("purl") for c in components}
    # npm 自带 SBOM 的锁文件视图包含全部 optional 平台；不只扫描本机 node_modules。
    for component in npm_bom.get("components", []):
        if component.get("purl") not in purls:
            components.append(component)
            purls.add(component.get("purl"))
    for component in components:
        if component.get("purl", "").startswith("pkg:cargo/"):
            value = licenses.get((component["name"], component["version"]))
            if value:
                component["licenses"] = [{"expression": value}]
    for name in ("pg_jieba", "cppjieba", "limonp"):
        source = lock["sources"][name]
        ref = f"pkg:generic/{name}@{source['commit']}"
        components.append({"type": "library", "name": name, "version": source["commit"],
                           "bom-ref": ref, "purl": ref,
                           "licenses": [{"license": {"id": source["license"]}}],
                           "hashes": [{"alg": "SHA-256", "content": source["sha256"]}],
                           "externalReferences": [{"type": "vcs", "url": source["repository"]}],
                           "properties": [{"name": "bbbcm:scope", "value": "postgres-test-image-source"}]})
    return bom


def license_inventory(bom):
    # 这是待复核清单；扫描器分类并不等于法律授权，不自动把未知资产标为通过。
    return [{"name": c["name"], "version": c.get("version"), "purl": c.get("purl"),
             "licenses": c.get("licenses", []),
             "status": "declared-needs-review" if c.get("licenses") else "unknown-needs-review"}
            for c in bom.get("components", [])]


def critical_high(report):
    return [v for result in report.get("Results", []) for v in result.get("Vulnerabilities", [])
            if v.get("Severity") in ("CRITICAL", "HIGH")]


def check_npm_coverage(npm_lock, bom):
    expected = {(p.get("name", path.rsplit("node_modules/", 1)[-1]), p["version"])
                for path, p in npm_lock["packages"].items() if path}
    actual = {(c["name"], c["version"]) for c in bom.get("components", [])
              if c.get("purl", "").startswith("pkg:npm/")}
    baseline.require(expected <= actual, "npm SBOM 缺少锁定包或版本（包括 optional/dev）")


def source_sbom(out):
    baseline.check_baseline()
    for tool in ("node", "syft"):
        baseline.verify_tool(tool)
    baseline.require(not baseline.run(["git", "status", "--porcelain"], capture=True), "全量 SBOM 需要干净已提交候选")
    with tempfile.TemporaryDirectory(prefix="bbbcm-sbom-") as temp:
        source = Path(temp) / "source"
        source.mkdir()
        archive = Path(temp) / "source.tar"
        with archive.open("wb") as stream:
            subprocess.run(["git", "archive", "HEAD"], cwd=ROOT, stdout=stream, check=True)
        baseline.extract(archive, source)
        syft_report = out / "source.syft.json"
        command([baseline.tool_path("syft"), "scan", "dir:" + str(source), "--config", CONFIG / "syft.yaml",
                 "--source-name", "bbbcm", "--source-version", baseline.run(["git", "rev-parse", "HEAD"], capture=True),
                 "-o", "syft-json=" + str(syft_report), "-o", "cyclonedx-json=" + str(out / "source.cdx.json")], syft_report)
        npm_bom = json.loads(baseline.npm(["sbom", "--package-lock-only", "--sbom-format=cyclonedx", "--include=dev", "--include=optional"], cwd=source, capture=True))
        baseline.write_json(out / "npm.cdx.json", npm_bom)
        metadata = json.loads(baseline.run(["cargo", "metadata", "--locked", "--offline", "--format-version=1"], cwd=source, capture=True))
        bom = enrich_bom(baseline.read_json(out / "source.cdx.json"), metadata, baseline.read_json(source / "supply-chain/lock.json"), npm_bom)
        # 一项未识别都不能用扫描成功冒充覆盖完整。
        cargo_lock = baseline.tomllib.loads((source / "Cargo.lock").read_text())["package"]
        actual = {(c["name"], c["version"]) for c in bom["components"] if c.get("purl", "").startswith("pkg:cargo/")}
        expected = {(p["name"], p["version"]) for p in cargo_lock if "source" in p}
        baseline.require(expected <= actual, "SBOM 缺少 Cargo 锁定依赖")
        check_npm_coverage(baseline.read_json(source / "package-lock.json"), npm_bom)
        baseline.write_json(out / "source.cdx.json", bom)
        baseline.write_json(out / "license-inventory.json", license_inventory(bom))
        # 原清单原样附带，不改写 A00 许可状态，也不把它当扫描通过的例外。
        (out / "a00-assets.json").write_bytes((source / "docs/visual/a00/assets.json").read_bytes())


def scan_licenses(sbom, out):
    baseline.verify_tool("trivy")
    report = out / "licenses.json"
    command([*trivy_args("license", report), sbom], report)
    baseline.write_json(out / "license-inventory.json", license_inventory(baseline.read_json(sbom)))


def db_snapshot():
    dbdir = baseline.CACHE / "trivy/db"
    paths = [dbdir / "trivy.db", dbdir / "metadata.json"]
    baseline.require(all(p.is_file() for p in paths), "漏洞库未准备；先运行 fetch-db 或恢复留存快照")
    return {p.name: baseline.sha256(p) for p in paths}


def scan_vulns(sbom, out):
    baseline.verify_tool("trivy")
    snapshot = db_snapshot()
    baseline.write_json(out / "database-sha256.json", snapshot)
    metadata = baseline.read_json(baseline.CACHE / "trivy/db/metadata.json")
    baseline.write_json(out / "database-metadata.json", metadata)
    report = out / "vulnerabilities.json"
    command([*trivy_args("vuln", report), sbom], report)
    baseline.require(db_snapshot() == snapshot, "扫描期间漏洞数据库变化")
    findings = critical_high(baseline.read_json(report))
    baseline.require(not findings, f"发现 {len(findings)} 个 Critical/High；全部严重度见本地报告")


def secrets(out, staged):
    baseline.verify_tool("gitleaks")
    report = out / "secrets.json"
    args = [baseline.tool_path("gitleaks"), "git", "--config", CONFIG / "gitleaks.toml",
            "--redact=100", "--no-banner", "--no-color", "--ignore-gitleaks-allow",
            "--gitleaks-ignore-path", CONFIG / "gitleaks.ignore",
            "--report-format", "json", "--report-path", report]
    args += ["--pre-commit", "--staged"] if staged else ["--log-opts=--all"]
    command([*args, ROOT], report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["sbom", "licenses", "vulns", "secrets", "fetch-db", "image-sbom"])
    parser.add_argument("--output", type=Path, required=True, help="新的本地报告目录，失败报告也保留")
    parser.add_argument("--sbom", type=Path)
    parser.add_argument("--staged", action="store_true")
    parser.add_argument("--archive", type=Path, help="docker save 的实际镜像归档")
    args = parser.parse_args()
    out = args.output.resolve()
    baseline.require(out.is_relative_to(baseline.CACHE.resolve()), "原始报告只能写入 .cache/supply-chain/；脱敏后再归档公开证据")
    out.mkdir(parents=True, exist_ok=False)
    record = {"command": args.command, "started": datetime.now(timezone.utc).isoformat(),
              "commit": baseline.run(["git", "rev-parse", "HEAD"], capture=True),
              "tools": {k: v["version"] for k, v in baseline.read_json(baseline.LOCK_PATH)["tools"].items()},
              "inputs": {p: baseline.sha256(ROOT / p) for p in ("Cargo.lock", "package-lock.json", "supply-chain/lock.json")}}
    try:
        if args.command == "sbom":
            source_sbom(out)
        elif args.command in ("licenses", "vulns"):
            baseline.require(args.sbom and args.sbom.is_file(), "需要 --sbom 指向实际 CycloneDX 文件")
            bom = baseline.read_json(args.sbom)
            baseline.require(bom.get("bomFormat") == "CycloneDX" and bom.get("components"), "拒绝空白或非 CycloneDX SBOM")
            record["sbom_sha256"] = baseline.sha256(args.sbom)
            (scan_licenses if args.command == "licenses" else scan_vulns)(args.sbom.resolve(), out)
        elif args.command == "secrets":
            record["scope"] = "staged" if args.staged else "all-git-history"
            if args.staged:
                diff = subprocess.check_output(["git", "diff", "--cached", "--binary"], cwd=ROOT)
                record["staged_diff_sha256"] = baseline.hashlib.sha256(diff).hexdigest()
            secrets(out, args.staged)
        elif args.command == "fetch-db":
            baseline.verify_tool("trivy")
            command([baseline.tool_path("trivy"), "image", "--download-db-only", "--no-progress",
                     "--skip-version-check", "--disable-telemetry", "--db-repository", "ghcr.io/aquasecurity/trivy-db:2",
                     "--cache-dir", baseline.CACHE / "trivy"], out / "download.json")
            baseline.write_json(out / "database-sha256.json", db_snapshot())
            (out / "database-metadata.json").write_bytes((baseline.CACHE / "trivy/db/metadata.json").read_bytes())
        else:
            baseline.verify_tool("syft")
            baseline.require(args.archive and args.archive.is_file(), "需要 --archive 指向实际 docker save 归档")
            record["image_archive_sha256"] = baseline.sha256(args.archive)
            report = out / "image.cdx.json"
            command([baseline.tool_path("syft"), "scan", "docker-archive:" + str(args.archive.resolve()),
                     "--config", CONFIG / "syft.yaml", "-o", "cyclonedx-json=" + str(report)], report)
        record["exit_code"] = 0
    except Exception:
        record["exit_code"] = 1
        raise
    finally:
        record["finished"] = datetime.now(timezone.utc).isoformat()
        record["files"] = {p.name: baseline.sha256(p) for p in out.iterdir() if p.is_file()}
        baseline.write_json(out / "run.json", record)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"扫描失败：{error}", file=sys.stderr)
        sys.exit(1)
