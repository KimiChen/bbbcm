#!/usr/bin/env python3
"""A01 按需资产入口，共用 M0-03 的 Node/npm/esbuild 与唯一根锁文件。"""
import argparse
import importlib.util
import subprocess
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("supply_chain", Path(__file__).with_name("supply-chain.py"))
baseline = importlib.util.module_from_spec(spec)
spec.loader.exec_module(baseline)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build", "test", "clean"])
    args = parser.parse_args()
    try:
        baseline.verify_tool("node")
        baseline.require(baseline.npm(["--version"], capture=True) ==
                         baseline.read_json(baseline.LOCK_PATH)["tools"]["node"]["npm"], "npm 版本不符")
        baseline.npm(["run", "assets:" + args.command])
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"资产入口失败：{error}", file=sys.stderr)
        sys.exit(1)
