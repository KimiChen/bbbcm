#!/usr/bin/env python3
"""WP-M0-03 的受控入口；Python 3.11+ 标准库，无自动版本解析。"""

import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "supply-chain/lock.json"
CACHE = ROOT / ".cache/supply-chain"
TOOLS = ROOT / ".local/tools"


def require(ok, message):
    if not ok:
        raise ValueError(message)


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(args, *, cwd=ROOT, env=None, capture=False):
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, check=True,
                            text=True, stdout=subprocess.PIPE if capture else None)
    return result.stdout.strip() if capture else None


def host_platform():
    arch = {"aarch64": "arm64", "arm64": "arm64", "x86_64": "amd64"}.get(platform.machine())
    value = f"{platform.system().lower()}/{arch}"
    require(value in read_json(LOCK_PATH)["tools"]["node"]["artifacts"], f"未锁定平台：{value}")
    return value


def artifact(item, offline=False):
    """只接受锁文件中的 HTTPS + SHA-256；缓存污染不自动覆盖。"""
    require(re.fullmatch(r"[0-9a-f]{64}", item["sha256"]), "无效的 SHA-256")
    require(item["url"].startswith("https://"), "下载必须使用 HTTPS")
    path = CACHE / "downloads" / item["sha256"]
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        require(not offline, f"离线缓存缺失：{item['url']}")
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temp:
            temp_path = Path(temp.name)
            try:
                with urllib.request.urlopen(item["url"], timeout=60) as response:
                    shutil.copyfileobj(response, temp)
                temp.flush()
                require(sha256(temp_path) == item["sha256"], f"下载摘要不符：{item['url']}")
                temp_path.replace(path)
            finally:
                temp_path.unlink(missing_ok=True)
    require(sha256(path) == item["sha256"], f"缓存摘要不符：{path.name}")
    return path


def extract(archive, destination):
    # Python 3.11.4+ 提供 data filter，拒绝路径穿越、设备和越界链接。
    with tarfile.open(archive) as tar:
        tar.extractall(destination, filter="data")


def tool_path(name):
    data = read_json(LOCK_PATH)["tools"][name]
    dest = TOOLS / name / data["version"] / host_platform().replace("/", "-")
    if name == "node":
        arch = "x64" if host_platform().endswith("amd64") else "arm64"
        dest /= f"node-v{data['version']}-{platform.system().lower()}-{arch}/bin"
    return dest / name


def install_tools(names, offline):
    lock = read_json(LOCK_PATH)
    for name in names:
        item = lock["tools"][name]["artifacts"][host_platform()]
        archive = artifact(item, offline)
        dest = TOOLS / name / lock["tools"][name]["version"] / host_platform().replace("/", "-")
        dest.mkdir(parents=True, exist_ok=True)
        extract(archive, dest)
        verify_tool(name)


def verify_tool(name):
    path = tool_path(name)
    require(path.is_file(), f"缺少 {name}，先运行 install-tools {name}")
    flag = "version" if name in ("syft", "gitleaks") else "--version"
    output = run([path, flag], capture=True)
    expected = read_json(LOCK_PATH)["tools"][name]["version"]
    require(re.search(rf"(?<![\d.])v?{re.escape(expected)}(?![\d.])", output), f"{name} 版本不符")
    print(f"{name} {expected}")


def npm(args, *, cwd=ROOT, capture=False):
    node = tool_path("node")
    env = dict(os.environ, PATH=str(node.parent) + os.pathsep + os.environ.get("PATH", ""))
    CACHE.mkdir(parents=True, exist_ok=True)
    # npm 11 不允许 user/global 同时指向同一个文件。
    global_config = CACHE / "npm-global.npmrc"
    global_config.write_text("# intentionally empty\n")
    return run([node.parent / "npm", "--userconfig=/dev/null", "--globalconfig=" + str(global_config), *args],
               cwd=cwd, env=env, capture=capture)


def npm_ci(offline):
    verify_tool("node")
    require(npm(["--version"], capture=True) == read_json(LOCK_PATH)["tools"]["node"]["npm"], "npm 版本不符")
    before = sha256(ROOT / "package-lock.json")
    args = ["ci", "--ignore-scripts", "--include=dev", "--include=optional", "--no-audit", "--no-fund",
            "--registry=https://registry.npmjs.org/"]
    if offline:
        args.append("--offline")
    npm(args)
    require(sha256(ROOT / "package-lock.json") == before, "npm ci 修改了锁文件")
    expected = read_json(ROOT / "package.json")["devDependencies"]["esbuild"]
    actual = run([tool_path("node"), "node_modules/esbuild/bin/esbuild", "--version"], capture=True)
    require(actual == expected, "esbuild 原生二进制版本不符")
    print(f"npm ci 锁文件未变，esbuild {actual}")


def check_baseline():
    lock = read_json(LOCK_PATH)
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    rust = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    require(lock["schema_version"] == 1, "不支持的锁文件格式")
    require(rust["channel"] == lock["rust"]["version"], "Rust 版本漂移")
    require(cargo["workspace"]["package"]["rust-version"] == lock["rust"]["msrv"], "MSRV 声明漂移")
    members = {tomllib.loads((ROOT / p / "Cargo.toml").read_text())["package"]["name"]
               for p in cargo["workspace"]["members"]}
    for package in packages:
        if "source" not in package:
            require(package["name"] in members, "Cargo 出现未登记路径依赖")
        else:
            require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index", "Cargo 非官方来源")
            require(re.fullmatch(r"[0-9a-f]{64}", package.get("checksum", "")), "Cargo 缺少 checksum")
    package = read_json(ROOT / "package.json")
    npm_lock = read_json(ROOT / "package-lock.json")
    node = lock["tools"]["node"]
    require((ROOT / ".node-version").read_text().strip() == node["version"] == package["engines"]["node"], "Node 版本漂移")
    require(package["engines"]["npm"] == node["npm"] and package["packageManager"] == "npm@" + node["npm"], "npm 版本漂移")
    require(npm_lock["lockfileVersion"] == 3, "npm 锁文件格式不是 v3")
    require(npm_lock["packages"][""]["devDependencies"] == package["devDependencies"], "npm 根依赖不一致")
    require(npm_lock["packages"][""]["engines"] == package["engines"], "npm 根 engines 不一致")
    for name, version in package["devDependencies"].items():
        require(re.fullmatch(r"\d+\.\d+\.\d+", version), "前端直接依赖必须精确版本")
        require(npm_lock["packages"]["node_modules/" + name]["version"] == version, "npm 解析版本漂移")
    for name, value in npm_lock["packages"].items():
        if not name:
            continue
        require(value.get("resolved", "").startswith("https://registry.npmjs.org/"), "npm 非官方来源")
        require(re.fullmatch(r"sha512-[A-Za-z0-9+/]+=*", value.get("integrity", "")), "npm 缺少 SHA-512 integrity")
    for name, item in lock["images"].items():
        pattern = r"16\.\d+-alpine3\.\d+" if name == "postgres" else r"8\.\d+\.\d+-bookworm"
        require(re.fullmatch(pattern, item["tag"]), f"镜像标签不精确：{name}")
        require(set(item["platforms"]) == {"linux/amd64", "linux/arm64"}, "镜像缺少平台")
        for digest in [item["index_digest"], *(p["digest"] for p in item["platforms"].values())]:
            require(re.fullmatch(r"sha256:[0-9a-f]{64}", digest), "无效的 OCI 摘要")
    for name, source in lock["sources"].items():
        if name == "pg_trgm":
            continue
        require(re.fullmatch(r"[0-9a-f]{40}", source["commit"]), "源码未锁定 commit")
        require(source["url"].endswith(source["commit"]), "源码 URL 未绑定 commit")
    artifacts = [a for tool in lock["tools"].values() for a in tool["artifacts"].values()]
    artifacts += [s for s in lock["sources"].values() if "url" in s]
    for data in lock["apk"].values():
        require({"g++", "gcc", "cmake", "make", "musl-dev"} <= {p["name"] for p in data["packages"]}, "编译包缺失")
        provided = {p["name"]: p["version"] for p in data["packages"]}
        for package in data["packages"]:
            for value in package["provides"]:
                name, _, version = value.partition("=")
                provided[name] = version
        for package in data["packages"]:
            for dependency in package["depends"]:
                name, exact, version = dependency.partition("=")
                require(name in provided, f"APK 依赖未锁定：{dependency}")
                require(not exact or provided[name] == version, f"APK 依赖版本不符：{dependency}")
        artifacts += data["packages"]
    for item in artifacts:
        require(item["url"].startswith("https://") and "/latest/" not in item["url"], "浮动或不安全 URL")
        require(re.fullmatch(r"[0-9a-f]{64}", item["sha256"]), "归档摘要缺失")
    require("cargo build" not in (ROOT / "scripts/dev-check.sh").read_text(), "DEC-039：dev-check 不得加入独立构建")
    print(f"基线一致：{len(members)} 个 Workspace 成员，{len(packages)-len(members)} 个 registry crate，"
          f"{len(npm_lock['packages'])-1} 个 npm 包，{len(artifacts)} 个锁定归档")


def prepare_postgres(target, offline):
    lock = read_json(LOCK_PATH)
    require(target in lock["images"]["postgres"]["platforms"], "未锁定的镜像平台")
    arch = {"linux/amd64": "x86_64", "linux/arm64": "aarch64"}[target]
    context = CACHE / "postgres" / arch
    require(not context.exists(), f"构建上下文已存在，请使用已有上下文或先将其移走：{context.relative_to(ROOT)}")
    # 所有下载校验完成后才生成上下文；不会在失败时留下半套可构建上下文。
    sources = [(name, artifact(item, offline)) for name, item in lock["sources"].items() if "url" in item]
    packages = [(item, artifact(item, offline)) for item in lock["apk"][arch]["packages"]]
    (context / "apks").mkdir(parents=True)
    (context / "runtime-apks").mkdir()
    sums = []
    for name, source in sources:
        path = context / f"{name}.tar.gz"
        shutil.copyfile(source, path)
        sums.append(f"{sha256(path)}  {path.name}")
    for item, source in packages:
        path = context / "apks" / item["file"]
        shutil.copyfile(source, path)
        sums.append(f"{sha256(path)}  apks/{path.name}")
        if item["name"] in {"musl", "libgcc", "libstdc++"}:
            shutil.copyfile(source, context / "runtime-apks" / item["file"])
            sums.append(f"{sha256(source)}  runtime-apks/{item['file']}")
    (context / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    shutil.copyfile(LOCK_PATH, context / "lock.json")
    shutil.copyfile(ROOT / "supply-chain/postgres/Dockerfile", context / "Dockerfile")
    image = lock["images"]["postgres"]
    ref = image["repository"] + "@" + image["platforms"][target]["digest"]
    args = ["docker", "build", "--network=none", "--platform", target,
            "--build-arg", "POSTGRES_IMAGE=" + ref, "--tag", "bbbcm-postgres:m0-03-" + arch,
            "--iidfile", str(context.relative_to(ROOT) / "image-id.txt"), str(context.relative_to(ROOT))]
    import shlex
    (context / "build-command.txt").write_text(shlex.join(args) + "\n")
    print(shlex.join(args))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check", "install-tools", "npm-ci", "npm-lock", "prepare-postgres", "image-ref"])
    parser.add_argument("tools", nargs="*", choices=["node", "syft", "trivy", "gitleaks"])
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--platform", default="linux/arm64", choices=["linux/arm64", "linux/amd64"])
    parser.add_argument("--image", choices=["postgres", "redis"], default="postgres")
    args = parser.parse_args()
    if args.command == "check":
        check_baseline()
    elif args.command == "install-tools":
        install_tools(args.tools or ["node", "syft", "trivy", "gitleaks"], args.offline)
    elif args.command == "npm-ci":
        npm_ci(args.offline)
    elif args.command == "npm-lock":
        verify_tool("node")
        require(npm(["--version"], capture=True) == read_json(LOCK_PATH)["tools"]["node"]["npm"], "npm 版本不符")
        npm(["install", "--package-lock-only", "--ignore-scripts", "--include=dev", "--include=optional",
             "--no-audit", "--no-fund", "--registry=https://registry.npmjs.org/"] + (["--offline"] if args.offline else []))
    elif args.command == "prepare-postgres":
        prepare_postgres(args.platform, args.offline)
    elif args.command == "image-ref":
        image = read_json(LOCK_PATH)["images"][args.image]
        print(image["repository"] + "@" + image["platforms"][args.platform]["digest"])


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"供应链入口失败：{error}", file=sys.stderr)
        sys.exit(1)
