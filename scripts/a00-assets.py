#!/usr/bin/env python3
"""Generate/check the A00 source-asset inventory, without building A01 dist output."""

from __future__ import annotations

import argparse
from fractions import Fraction
import hashlib
import json
from pathlib import Path
import re
import struct
import sys
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
STATIC = Path("apps/web/static")
MANIFEST = Path("docs/visual/a00/assets.json")
RIGHTS = Path("docs/visual/a00/licenses")
METADATA_FILES = (
    RIGHTS / "font-assets.json",
    RIGHTS / "brand-assets.json",
    RIGHTS / "icons-assets.json",
    RIGHTS / "feather-icons-assets.json",
    RIGHTS / "reference-ui-pending-assets.json",
    RIGHTS / "localized-brand-assets.json",
    RIGHTS / "localized-og-assets.json",
)
STYLESHEETS = {
    "css/profile.css": "A00 公开个人资料页稳定夹具样式",
    "css/site.css": "A00 公共外壳、设计 token 与组件样式",
    "css/pages.css": "A00 公开页面与稳定夹具样式",
    "css/fonts.css": "A00 本地 Space Mono 字体声明",
}
MIMES = {".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2",
         ".png": "image/png", ".ico": "image/x-icon"}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def read_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def local_path(root: Path, value: str) -> Path:
    """Reject paths outside the checkout, including symlinks to external evidence."""
    relative = Path(value)
    require(not relative.is_absolute() and ".." not in relative.parts, f"非法路径：{value}")
    result = root / relative
    require(result.resolve().is_relative_to(root.resolve()), f"路径越界：{value}")
    return result


def source_files(root: Path) -> set[str]:
    result = set()
    for path in (root / STATIC).rglob("*"):
        relative = path.relative_to(root / STATIC)
        # A01 build output and empty-directory sentinels are not source assets.
        if relative.parts[0] == "dist" or path.name == ".gitkeep":
            continue
        require(not path.is_symlink(), f"源资产不能是符号链接：{path.relative_to(root)}")
        if path.is_file():
            result.add(path.relative_to(root).as_posix())
    return result


def load_metadata(root: Path, sources: set[str]) -> dict[str, dict]:
    by_path: dict[str, dict] = {}
    for metadata_path in METADATA_FILES:
        rows = read_json(root / metadata_path)
        require(isinstance(rows, list), f"资产元数据必须是数组：{metadata_path}")
        for row in rows:
            require(isinstance(row, dict), f"无效元数据项：{metadata_path}")
            path = row.get("path")
            require(isinstance(path, str) and path not in by_path, f"资产路径缺失或重复：{path}")
            local_path(root, path)
            by_path[path] = dict(row)
    for relative, accessible_name in STYLESHEETS.items():
        path = (STATIC / relative).as_posix()
        if path in sources:
            require(path not in by_path, f"样式表元数据重复：{path}")
            by_path[path] = {
                "logical_name": "a00-" + Path(relative).stem,
                "path": path,
                "mime": "text/css",
                "width": None,
                "height": None,
                "dimensions_note": "CSS 样式表没有固有图像尺寸。",
                "theme": "light/dark",
                "accessible_name": accessible_name,
                "source": "本项目自制：依据本轮公开可观察的设计 token 与组件行为重新编写。",
                "license": "AGPL-3.0-only",
                "redistributable": "yes",
                "evidence": ["LICENSE", "docs/visual/a00/licenses/asset-rights.md"],
            }
    missing = sources - by_path.keys()
    extra = by_path.keys() - sources
    require(not missing, "静态源文件缺少权利元数据：" + ", ".join(sorted(missing)))
    require(not extra, "权利元数据指向不存在的静态源文件：" + ", ".join(sorted(extra)))
    return by_path


def validate_feather(root: Path, row: dict, evidence: list[str]) -> list[str]:
    """Validate only the two reviewed Feather affine adaptations, not arbitrary MIT SVGs."""
    logical = row["logical_name"]
    settings = {
        "ui-menu": ("menu", 19, Fraction(3, 2),
                    "896faf98953d6732f4286114a14401a0c881674d0d6f41d8253db24193ac51a5"),
        "ui-chevron-up": ("chevron-up", 16, Fraction(0),
                          "185e8665dd1cb409377f28f23323b35f098fcfcd6b657d8f85991f0a9bc1dcfe"),
    }
    require(logical in settings, f"未确认 Feather 图标：{logical}")
    icon, size, translation, upstream_hash = settings[logical]
    require(row["path"] == f"apps/web/static/img/{logical}.svg", "Feather 改编图标路径未经确认")
    required = [(RIGHTS / name).as_posix() for name in
                ("feather-MIT.txt", "feather-source.json", "feather-geometry.json", f"feather-upstream/{icon}.svg.json")]
    require(all(path in evidence for path in required), "Feather 改编缺少专属许可/原字节/变换证据")
    license_bytes = local_path(root, required[0]).read_bytes()
    require(hashlib.sha256(license_bytes).hexdigest() ==
            "308028e93fcf84972523cdf6e616f73168546b4953895f516d01287f16fe7bee",
            "Feather MIT 原文 SHA-256 不符")
    license_text = license_bytes.decode("utf-8")
    require("The MIT License (MIT)" in license_text and "Copyright (c) 2013-2023 Cole Bemis" in license_text,
            "Feather MIT 许可/版权声明不完整")
    provenance = read_json(local_path(root, required[1]))
    require(isinstance(provenance, dict), "Feather 来源证据必须是对象")
    commit = "3dc050d97405062eba78aa57115c0a15c63abdaa"
    url = f"https://raw.githubusercontent.com/feathericons/feather/{commit}/icons/{icon}.svg"
    require(provenance.get("upstream_repository") == "https://github.com/feathericons/feather"
            and provenance.get("upstream_commit") == commit
            and provenance.get("license_sha256") == hashlib.sha256(license_bytes).hexdigest()
            and row.get("source_url") == url, "Feather 固定上游/版本/许可来源不符")
    source = next((item for item in provenance.get("icons", []) if item.get("logical_name") == logical), None)
    require(source is not None and source.get("source_evidence") == required[3]
            and source.get("upstream_sha256") == upstream_hash and source.get("source_url") == url
            and source.get("distributed_sha256") == row["sha256"], "Feather 来源与分发哈希链不完整")
    source_container = read_json(local_path(root, required[3]))
    require(isinstance(source_container, dict) and source_container.get("encoding") == "UTF-8"
            and isinstance(source_container.get("source_text"), str), "Feather 原字节 JSON 容器无效")
    source_bytes = source_container["source_text"].encode("utf-8")
    require(hashlib.sha256(source_bytes).hexdigest() == upstream_hash
            and source_container.get("sha256") == upstream_hash
            and source_container.get("source_url") == url, "Feather 官方源 SVG 原字节不符")
    geometry = read_json(local_path(root, required[2]))
    require(isinstance(geometry, dict), "Feather 变换证据必须是对象")
    proof = next((item for item in geometry.get("icons", []) if item.get("logical_name") == logical), None)
    expected_modifications = {"coordinate_scale": "2/3", "coordinate_translate": [float(translation)] * 2,
                              "output_view_box": [0, 0, size, size], "output_stroke_width": 1.7,
                              "retain_round_caps_joins": True}
    require(proof is not None and proof.get("source_commit") == commit
            and proof.get("modifications") == expected_modifications
            and bool(proof.get("ownership_boundary")) and bool(row.get("modification")),
            "Feather 精确变换或改编归属说明不完整")
    expected = ET.fromstring(source_bytes)
    expected.set("width", str(size))
    expected.set("height", str(size))
    expected.set("viewBox", f"0 0 {size} {size}")
    expected.set("stroke-width", "1.7")
    vertices = []

    def transformed(value: str) -> str:
        return str(float(Fraction(value) * Fraction(2, 3) + translation)).removesuffix(".0")

    for element in expected:
        if element.tag.endswith("}line"):
            for key in ("x1", "y1", "x2", "y2"):
                element.set(key, transformed(element.attrib[key]))
            vertices.append(sorted([[float(element.attrib["x1"]), float(element.attrib["y1"])],
                                    [float(element.attrib["x2"]), float(element.attrib["y2"])]]))
        elif element.tag.endswith("}polyline"):
            coordinates = [transformed(value) for value in element.attrib["points"].split()]
            element.set("points", " ".join(coordinates))
            vertices.append([[float(coordinates[index]), float(coordinates[index + 1])]
                             for index in range(0, len(coordinates), 2)])
        else:
            raise ValueError("Feather 固定源包含未确认的图形节点")

    def canonical(element):
        require(not (element.text or "").strip() and not (element.tail or "").strip(),
                "Feather 图形出现未确认的文字")
        return (element.tag, sorted(element.attrib.items()), [canonical(child) for child in element])

    actual = ET.parse(local_path(root, row["path"])).getroot()
    require(canonical(actual) == canonical(expected), "Feather 分发 SVG 不等于已批准的精确变换结果")
    reference_vertices = ([[[3.5, 5.5], [15.5, 5.5]], [[3.5, 9.5], [15.5, 9.5]],
                           [[3.5, 13.5], [15.5, 13.5]]] if logical == "ui-menu"
                          else [[[4, 10], [8, 6], [12, 10]]])
    equivalent = sorted(vertices) == sorted(reference_vertices) if logical == "ui-menu" else vertices[0][::-1] == reference_vertices[0]
    require(equivalent and proof.get("transformed_geometry") == vertices
            and proof.get("observed_reference_geometry") == reference_vertices
            and proof.get("all_visible_vertex_coordinates_exact") is True
            and proof.get("stroke_width_equal") is True
            and proof.get("caps") == "round" and proof.get("joins") == "round",
            "Feather 变换与参考目标坐标/描边比较证据不符")
    return evidence


def validate_feather_search(root: Path, row: dict, evidence: list[str]) -> list[str]:
    """Accept only the reviewed independent circle/handle transformations."""
    require(row["logical_name"] == "ui-search" and row["path"] == "apps/web/static/img/ui-search.svg",
            "Feather 搜索图标路径未经确认")
    required = [(RIGHTS / name).as_posix() for name in (
        "feather-MIT.txt", "feather-search-source.json", "feather-search-geometry.json",
        "feather-upstream/search.svg.json", "feather-search-provenance.md")]
    require(all(path in evidence for path in required), "Feather 搜索缺少专属许可/原字节/配方证据")
    license_hash = "308028e93fcf84972523cdf6e616f73168546b4953895f516d01287f16fe7bee"
    require(hashlib.sha256(local_path(root, required[0]).read_bytes()).hexdigest() == license_hash,
            "Feather 搜索 MIT 原文 SHA-256 不符")
    commit = "3dc050d97405062eba78aa57115c0a15c63abdaa"
    url = f"https://raw.githubusercontent.com/feathericons/feather/{commit}/icons/search.svg"
    upstream_hash = "5f27d6498c8d1746ef65458aedc42b51052a4ef1c52fb5d0cd900c19676a9db3"
    provenance = read_json(local_path(root, required[1]))
    require(isinstance(provenance, dict)
            and provenance.get("upstream_repository") == "https://github.com/feathericons/feather"
            and provenance.get("upstream_commit") == commit and provenance.get("license") == "MIT"
            and provenance.get("license_url") == f"https://raw.githubusercontent.com/feathericons/feather/{commit}/LICENSE"
            and provenance.get("license_sha256") == license_hash
            and provenance.get("copyright") == "Copyright (c) 2013-2023 Cole Bemis"
            and provenance.get("source_url") == url and row.get("source_url") == url
            and provenance.get("source_evidence") == required[3]
            and provenance.get("upstream_sha256") == upstream_hash
            and provenance.get("distributed_sha256") == row["sha256"],
            "Feather 搜索固定版本/许可/来源与分发哈希链不符")
    source = read_json(local_path(root, required[3]))
    require(isinstance(source, dict) and source.get("encoding") == "UTF-8"
            and isinstance(source.get("source_text"), str), "Feather 搜索原字节容器无效")
    source_bytes = source["source_text"].encode("utf-8")
    require(hashlib.sha256(source_bytes).hexdigest() == upstream_hash
            and source.get("sha256") == upstream_hash and source.get("source_url") == url,
            "Feather 搜索官方源 SVG 原字节不符")
    proof = read_json(local_path(root, required[2]))
    recipe = {
        "circle": {"scale": "9/16", "translate": ["-3/16", "-3/16"]},
        "handle": {"scale": "70/87", "translate": ["-113/29", "-113/29"]},
        "output_view_box": [0, 0, 14, 14], "output_stroke_width": "1.4",
        "retain_round_caps_joins": True,
    }
    require(isinstance(proof, dict) and proof.get("logical_name") == "ui-search"
            and proof.get("source_commit") == commit and proof.get("recipe") == recipe
            and bool(proof.get("ownership_boundary")) and bool(row.get("modification")),
            "Feather 搜索独立组件变换配方或归属说明不符")
    expected = ET.fromstring(source_bytes)
    namespace = "{http://www.w3.org/2000/svg}"
    require([child.tag for child in expected] == [namespace + "circle", namespace + "line"],
            "Feather 搜索原始组件结构未经确认")
    circle, handle = expected
    source_geometry = {
        "circle": {"center": [float(circle.attrib[key]) for key in ("cx", "cy")],
                   "radius": float(circle.attrib["r"])},
        "handle": [[float(handle.attrib[axis + number]) for axis in ("x", "y")] for number in ("1", "2")],
    }

    def value(number: Fraction) -> str:
        # All reviewed final coordinates are integers or halves; no rounding.
        require(number.denominator in (1, 2), "Feather 搜索变换产生未确认的非半整数坐标")
        return str(float(number)).removesuffix(".0")

    for key in ("cx", "cy"):
        circle.set(key, value(Fraction(circle.attrib[key]) * Fraction(9, 16) - Fraction(3, 16)))
    circle.set("r", value(Fraction(circle.attrib["r"]) * Fraction(9, 16)))
    for key in ("x1", "y1", "x2", "y2"):
        handle.set(key, value(Fraction(handle.attrib[key]) * Fraction(70, 87) - Fraction(113, 29)))
    expected.set("width", "14")
    expected.set("height", "14")
    expected.set("viewBox", "0 0 14 14")
    expected.set("stroke-width", "1.4")

    def canonical(element):
        require(not (element.text or "").strip() and not (element.tail or "").strip(),
                "Feather 搜索图形出现未确认的文字")
        return (element.tag, sorted(element.attrib.items()), [canonical(child) for child in element])

    actual = ET.parse(local_path(root, row["path"])).getroot()
    require(canonical(actual) == canonical(expected), "Feather 搜索 SVG 不等于固定组件变换结果")
    transformed = {
        "circle": {"center": [float(circle.attrib[key]) for key in ("cx", "cy")],
                   "radius": float(circle.attrib["r"])},
        "handle": [[float(handle.attrib[axis + number]) for axis in ("x", "y")] for number in ("1", "2")],
    }
    reference = {"circle": {"center": [6, 6], "radius": 4.5}, "handle": [[9.5, 9.5], [13, 13]]}
    require(transformed["circle"] == reference["circle"] and transformed["handle"][::-1] == reference["handle"]
            and proof.get("source_geometry") == source_geometry
            and proof.get("transformed_geometry") == transformed
            and proof.get("observed_reference_geometry") == reference
            and proof.get("all_visible_geometry_exact") is True and proof.get("stroke_width_equal") is True,
            "Feather 搜索圆/手柄目标几何或比较证据不符")
    return evidence


def exact_path_commands(data: str) -> list[tuple]:
    """Normalize reviewed SVG paths with exact rationals, including compact arc flags."""
    offset = 0
    x = y = Fraction(0)
    start = (x, y)
    command = previous = control = None
    output = []

    def number(flag=False):
        nonlocal offset
        while offset < len(data) and (data[offset].isspace() or data[offset] == ","):
            offset += 1
        pattern = r"[01]" if flag else r"[-+]?(?:\d*\.\d+|\d+\.?\d*)(?:[eE][-+]?\d+)?"
        match = re.match(pattern, data[offset:])
        require(match is not None, "Feather 路径数值无效")
        offset += len(match[0])
        return Fraction(match[0])

    while offset < len(data):
        while offset < len(data) and (data[offset].isspace() or data[offset] == ","):
            offset += 1
        if offset == len(data):
            break
        if data[offset].isalpha():
            command = data[offset]
            offset += 1
        require(command is not None, "Feather 路径命令缺失")
        kind = command.upper()
        relative = command.islower()

        def pair(a, b):
            return (a + x, b + y) if relative else (a, b)

        if kind == "Z":
            output.append(("Z",))
            x, y = start
            control = command = None
            previous = kind
            continue
        counts = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "S": 4, "A": 7}
        require(kind in counts, "Feather 路径包含未经确认的命令")
        args = [number(kind == "A" and index in (3, 4)) for index in range(counts[kind])]
        if kind in ("M", "L"):
            x, y = pair(*args)
            output.append((kind, x, y))
            control = None
            if kind == "M":
                start = (x, y)
                command = "l" if relative else "L"
        elif kind == "H":
            x = x + args[0] if relative else args[0]
            output.append(("L", x, y))
            control = None
        elif kind == "V":
            y = y + args[0] if relative else args[0]
            output.append(("L", x, y))
            control = None
        elif kind in ("C", "S"):
            if kind == "C":
                first, second, end = pair(*args[:2]), pair(*args[2:4]), pair(*args[4:])
            else:
                first = (2 * x - control[0], 2 * y - control[1]) if previous in ("C", "S") and control else (x, y)
                second, end = pair(*args[:2]), pair(*args[2:])
            output.append(("C", *first, *second, *end))
            x, y = end
            control = second
        elif kind == "A":
            end = pair(*args[-2:])
            output.append(("A", *args[:5], *end))
            x, y = end
            control = None
        previous = kind
    return output


def validate_feather_shapes(root: Path, row: dict, evidence: list[str]) -> list[str]:
    """Rebuild only the two approved topology-preserving parameter adaptations."""
    settings = {
        "ui-eye": ("eye", "3e6c211d83ca5a71baec90ec51a97ff52ce1df17307e3c56254958bc5e54daaf",
                   {"half_width": 11, "half_height": 8, "outer_control_offset": 4},
                   {"half_width": "19/2", "half_height": 6, "outer_control_offset": "7/2"},
                   {"center": [12, 12], "pupil_radius": 3}),
        "ui-chat": ("message-square", "6b348b2fb5661dd43e4cedc32c167a4b389809660037f91cf6351667967202d9",
                    {"corner_radius": 2, "bottom_edge": 17, "tail_base_x": 7, "tail_tip_y": 21},
                    {"corner_radius": 4, "bottom_edge": 19, "tail_base_x": 8, "tail_tip_y": 22},
                    {"left_edge": 3, "right_edge": 21, "top_edge": 3}),
    }
    logical = row["logical_name"]
    require(logical in settings and row["path"] == f"apps/web/static/img/{logical}.svg",
            "Feather 参数改编图标路径未经确认")
    icon, upstream_hash, before, after, fixed = settings[logical]
    required = [(RIGHTS / name).as_posix() for name in (
        "feather-MIT.txt", "feather-shape-source.json", "feather-shape-geometry.json",
        f"feather-upstream/{icon}.svg.json", "feather-shape-provenance.md")]
    require(all(path in evidence for path in required), "Feather 参数改编缺少专属证据")
    commit = "3dc050d97405062eba78aa57115c0a15c63abdaa"
    url = f"https://raw.githubusercontent.com/feathericons/feather/{commit}/icons/{icon}.svg"
    license_hash = "308028e93fcf84972523cdf6e616f73168546b4953895f516d01287f16fe7bee"
    require(hashlib.sha256(local_path(root, required[0]).read_bytes()).hexdigest() == license_hash,
            "Feather 参数改编 MIT 原文 SHA-256 不符")
    provenance = read_json(local_path(root, required[1]))
    require(isinstance(provenance, dict) and provenance.get("upstream_commit") == commit
            and provenance.get("upstream_repository") == "https://github.com/feathericons/feather"
            and provenance.get("license") == "MIT" and provenance.get("license_sha256") == license_hash
            and provenance.get("license_url") == f"https://raw.githubusercontent.com/feathericons/feather/{commit}/LICENSE"
            and provenance.get("copyright") == "Copyright (c) 2013-2023 Cole Bemis",
            "Feather 参数改编固定上游/许可链不符")
    source_record = next((item for item in provenance.get("icons", []) if item.get("logical_name") == logical), None)
    require(source_record is not None and source_record.get("source_url") == url
            and row.get("source_url") == url and source_record.get("source_evidence") == required[3]
            and source_record.get("upstream_sha256") == upstream_hash
            and source_record.get("distributed_sha256") == row["sha256"],
            "Feather 参数改编原始/分发哈希链不符")
    container = read_json(local_path(root, required[3]))
    require(isinstance(container, dict) and container.get("encoding") == "UTF-8"
            and isinstance(container.get("source_text"), str), "Feather 参数改编原字节容器无效")
    source_bytes = container["source_text"].encode("utf-8")
    require(container.get("sha256") == upstream_hash and container.get("source_url") == url
            and hashlib.sha256(source_bytes).hexdigest() == upstream_hash,
            "Feather 参数改编官方源 SVG 原字节不符")
    geometry = read_json(local_path(root, required[2]))
    require(isinstance(geometry, dict), "Feather 参数改编几何证据无效")
    proof = next((item for item in geometry.get("icons", []) if item.get("logical_name") == logical), None)
    require(proof is not None and proof.get("source_commit") == commit
            and proof.get("source_parameters") == before and proof.get("target_parameters") == after
            and proof.get("fixed_parameters") == fixed and proof.get("stroke_width") == "2"
            and bool(proof.get("ownership_boundary")) and bool(row.get("modification")),
            "Feather 参数改编原始/目标参数或归属说明不符")

    def value(number):
        number = Fraction(number)
        require(number.denominator in (1, 2), "Feather 参数改编出现非半整数坐标")
        return str(float(number)).removesuffix(".0")

    def path_from_parameters(parameters):
        if logical == "ui-eye":
            width, height, offset = (Fraction(parameters[name]) for name in
                                     ("half_width", "half_height", "outer_control_offset"))
            values = (offset, -height, width, -height, width, height, width, height,
                      -offset, height, -width, height, -width, -height, -width, -height)
            return f"M{value(12 - width)} 12s" + " ".join(value(number) for number in values) + "z"
        radius, bottom, tail, tip = (Fraction(parameters[name]) for name in
                                    ("corner_radius", "bottom_edge", "tail_base_x", "tail_tip_y"))
        return (f"M21 {value(bottom - radius)}a{value(radius)} {value(radius)} 0 0 1 {value(-radius)} {value(radius)}"
                f"H{value(tail)}l{value(3 - tail)} {value(tip - bottom)}V{value(3 + radius)}"
                f"a{value(radius)} {value(radius)} 0 0 1 {value(radius)} {value(-radius)}h{value(18 - 2 * radius)}"
                f"a{value(radius)} {value(radius)} 0 0 1 {value(radius)} {value(radius)}z")

    expected = ET.fromstring(source_bytes)
    namespace = "{http://www.w3.org/2000/svg}"
    target_path = expected.find(namespace + "path")
    require(target_path is not None and exact_path_commands(path_from_parameters(before)) ==
            exact_path_commands(target_path.attrib["d"]), "Feather 参数化不能还原固定官方路径")
    target_path.set("d", path_from_parameters(after))

    def canonical(element):
        require(not (element.text or "").strip() and not (element.tail or "").strip(),
                "Feather 参数改编包含未确认的文字")
        return (element.tag, sorted(element.attrib.items()), [canonical(child) for child in element])

    actual = ET.parse(local_path(root, row["path"])).getroot()
    require(canonical(actual) == canonical(expected), "Feather 参数改编 SVG 不等于独立重算结果")
    # Exact current anonymous reference geometry, expanded to absolute path commands.
    reference = ([("M", 2.5, 12), ("C", 2.5, 12, 6, 6, 12, 6),
                  ("C", 18, 6, 21.5, 12, 21.5, 12), ("C", 21.5, 12, 18, 18, 12, 18),
                  ("C", 6, 18, 2.5, 12, 2.5, 12), ("Z",)] if logical == "ui-eye" else
                 [("M", 21, 15), ("A", 4, 4, 0, 0, 1, 17, 19), ("L", 8, 19), ("L", 3, 22),
                  ("L", 3, 7), ("A", 4, 4, 0, 0, 1, 7, 3), ("L", 17, 3),
                  ("A", 4, 4, 0, 0, 1, 21, 7), ("Z",)])
    commands = exact_path_commands(target_path.attrib["d"])
    serializable = [[part if isinstance(part, str) else float(part) for part in command] for command in commands]
    require(commands == reference and proof.get("observed_reference_commands") == serializable
            and proof.get("source_parameterization_reconstructs_official_path") is True
            and proof.get("all_visible_geometry_exact") is True,
            "Feather 参数改编与参考绝对几何证据不符")
    if logical == "ui-eye":
        pupil = expected.find(namespace + "circle")
        require(pupil is not None and pupil.attrib == {"cx": "12", "cy": "12", "r": "3"},
                "Feather 浏览图标瞳孔发生未经确认的修改")
    return evidence


def validate_evidence(root: Path, row: dict) -> list[str]:
    path = row["path"]
    license_name = row.get("license")
    require(license_name in {"AGPL-3.0-only", "OFL-1.1", "MIT", "unconfirmed"}, f"未获支持的许可证状态：{path}")
    evidence = row.get("evidence")
    require(isinstance(evidence, list) and bool(evidence), f"缺少许可证据：{path}")
    for reference in evidence:
        require(isinstance(reference, str), f"无效许可证据路径：{path}")
        evidence_path = local_path(root, reference)
        require(evidence_path.is_file() and evidence_path.stat().st_size > 0,
                f"许可证据文件不存在或为空：{reference}")
    if license_name == "unconfirmed":
        require(row.get("redistributable") == "pending"
                and row.get("local_versioning_decision") == "DEC-045"
                and row.get("public_release") == "not-approved"
                and "plan/06-decisions-and-risks.md" in evidence,
                f"未知许可必须完整记录 DEC-045 本机版本化与未批准公开发布的边界：{path}")
        require("### `DEC-045`" in (root / "plan/06-decisions-and-risks.md").read_text(),
                "缺少本机资产版本化决定")
        require(isinstance(row.get("source_url"), str) and row["source_url"].startswith("https://")
                and re.fullmatch(r"[a-f0-9]{64}", row.get("sha256", "")) is not None,
                f"未知许可资产必须固定来源与分发 SHA-256：{path}")
        if row["logical_name"] in {"ui-user", "ui-document", "ui-forum", "oauth-google", "oauth-github", "oauth-x"}:
            source_path = (RIGHTS / "reference-ui-pending-source.json").as_posix()
            require(source_path in evidence, "参考 UI 缺少本轮公开来源证据")
            source = next((item for item in read_json(root / source_path)["assets"]
                           if item["logical_name"] == row["logical_name"]), None)
            require(source is not None and source["distributed_sha256"] == row["sha256"]
                    and source["distributed_path"] == path and source["source_url"] == row["source_url"]
                    and hashlib.sha256(source["source_svg_utf8"].encode()).hexdigest() == source["source_sha256"],
                    "参考 UI 来源/分发哈希链不一致")
        else:
            require(row["logical_name"] in {"brand-symbol", "favicon-16", "favicon-32", "app-icon-180", "favicon-ico",
                                           "og-image-svg", "og-image", "email-header"},
                    f"未登记的 A00 未知许可资产：{path}")
            require(any(reference.endswith(("brand-localization.json", "og-localization.json")) for reference in evidence),
                    "本地化品牌缺少生成及来源证据")
            proof_path = next(reference for reference in evidence if reference.endswith(("brand-localization.json", "og-localization.json")))
            proof = read_json(root / proof_path)
            output = next((item for item in proof.get("outputs", []) if item.get("logical_name") == row["logical_name"]), None)
            require(output is not None and output.get("path") == path and output.get("sha256") == row["sha256"],
                    "本地化品牌生成记录与分发哈希不一致")
        return evidence
    require(row.get("redistributable") == "yes", f"已知许可未确认可再分发：{path}")
    if license_name == "MIT" and row["logical_name"] in {"ui-menu", "ui-chevron-up"}:
        return validate_feather(root, row, evidence)
    if license_name == "MIT" and row["logical_name"] == "ui-search":
        return validate_feather_search(root, row, evidence)
    if license_name == "MIT" and row["logical_name"] in {"ui-eye", "ui-chat"}:
        return validate_feather_shapes(root, row, evidence)
    if license_name == "AGPL-3.0-only":
        require("LICENSE" in evidence, f"自有资产未关联项目许可证：{path}")
        license_text = (root / "LICENSE").read_text(encoding="utf-8")
        require("GNU AFFERO GENERAL PUBLIC LICENSE" in license_text and "Version 3" in license_text,
                "项目 LICENSE 不是预期 AGPL-3.0 原文")
    elif license_name == "OFL-1.1":
        ofl_path = (RIGHTS / "space-mono-OFL-1.1.txt").as_posix()
        source_path = (RIGHTS / "space-mono-source.json").as_posix()
        require(ofl_path in evidence and source_path in evidence, f"字体缺少 OFL 或来源证据：{path}")
        ofl_bytes = local_path(root, ofl_path).read_bytes()
        ofl_text = ofl_bytes.decode("utf-8")
        require("SIL OPEN FONT LICENSE Version 1.1" in ofl_text
                and "Copyright 2016 The Space Mono Project Authors" in ofl_text,
                "字体 OFL 版权或许可原文不完整")
        provenance = read_json(local_path(root, source_path))
        require(isinstance(provenance, dict), "字体来源证据必须是对象")
        require(hashlib.sha256(ofl_bytes).hexdigest() == provenance.get("local_license_sha256"),
                "OFL 本地分发许可证 SHA-256 与来源证据不一致")
        normalization = provenance.get("license_normalization")
        require(isinstance(normalization, dict)
                and normalization.get("operation") == "strip-trailing-ascii-space-only"
                and normalization.get("changed_lines") == [{"line": 21, "removed_suffix_hex": "20"}]
                and normalization.get("preserve_license_text") is True
                and normalization.get("preserve_line_endings") is True
                and isinstance(normalization.get("description"), str)
                and bool(normalization["description"].strip()),
                "OFL 缺少准确的单行尾空格规范化说明")
        # Reconstruct the upstream bytes offline, proving that no license wording changed.
        license_lines = ofl_bytes.splitlines(keepends=True)
        require(len(license_lines) >= 21 and all(line.endswith(b"\n") for line in license_lines)
                and all(line[:-1] == line[:-1].rstrip(b" \t\r") for line in license_lines),
                "OFL 本地分发副本必须使用 LF 且无行尾空白")
        license_lines[20] = license_lines[20][:-1] + b" \n"
        upstream_hash = "8e4ee42b2553e1e01504e61cb0d46d148cd8c9e5eacaa3622a7df2d4f2955b9f"
        require(provenance.get("upstream_license_sha256") == upstream_hash
                and hashlib.sha256(b"".join(license_lines)).hexdigest() == upstream_hash,
                "OFL 规范化副本无法还原到已确认的上游原始 SHA-256")
        require(row.get("reference_bytes_identical") is True
                and row.get("reference_sha256") == row.get("sha256"),
                f"字体缺少与当前公开参考的字节一致证据：{path}")
    else:
        # MIT is accepted only for these pinned, unmodified Line MD source files.
        mit_path = (RIGHTS / "line-md-MIT.txt").as_posix()
        source_path = (RIGHTS / "line-md-source.json").as_posix()
        comparison_path = (RIGHTS / "ui-icon-comparison.json").as_posix()
        require(all(reference in evidence for reference in (mit_path, source_path, comparison_path)),
                f"MIT 图标缺少专属上游、许可或参考匹配证据：{path}")
        mit_bytes = local_path(root, mit_path).read_bytes()
        mit_text = mit_bytes.decode("utf-8")
        require("Copyright 2020 Vjacheslav Trushkin" in mit_text
                and "Permission is hereby granted, free of charge" in mit_text
                and "The above copyright notice and this permission notice" in mit_text,
                "Material Line Icons MIT 版权或许可原文不完整")
        provenance = read_json(local_path(root, source_path))
        require(isinstance(provenance, dict), "MIT 图标来源证据必须是对象")
        require(hashlib.sha256(mit_bytes).hexdigest() == provenance.get("license_sha256"),
                "Material Line Icons MIT 原文 SHA-256 与来源证据不一致")
        require(provenance.get("upstream_repository") == "https://github.com/cyberalien/line-md",
                "MIT 图标不是已确认的 Material Line Icons 上游")
        pinned_commit = "2ed22555cee9c1e50d4269865681d01ee8cffd7c"
        require(provenance.get("upstream_commit") == pinned_commit, "MIT 图标上游版本未经本轮确认")
        expected_icons = {
            "theme-sun": "moon-filled-to-sunny-filled-loop-transition",
            "theme-moon": "moon-filled-loop",
        }
        logical_name = row["logical_name"]
        require(logical_name in expected_icons, f"未确认 MIT 图标：{logical_name}")
        icon_name = expected_icons[logical_name]
        expected_url = f"https://raw.githubusercontent.com/cyberalien/line-md/{pinned_commit}/svg/{icon_name}.svg"
        require(row.get("source_url") == expected_url, f"MIT 图标来源 URL 与固定版本不符：{path}")
        source_icon = next((icon for icon in provenance.get("icons", [])
                            if icon.get("logical_name") == logical_name), None)
        require(source_icon is not None and source_icon.get("sha256") == row.get("sha256")
                and source_icon.get("source_url") == expected_url,
                f"MIT 图标不是未修改的已确认上游源文件：{path}")
        comparison = read_json(local_path(root, comparison_path))
        require(isinstance(comparison, dict), "MIT 图标对比证据必须是对象")
        matched = next((icon for icon in comparison.get("matches", [])
                        if icon.get("logical_name") == logical_name), None)
        require(matched is not None and matched.get("canonical_tree_equal") is True
                and matched.get("all_geometry_paths_matched") is True
                and matched.get("animation_attributes_equal") is True,
                f"MIT 图标缺少完整路径/动画对比证据：{path}")
        reference_svg = local_path(root, matched["reference_evidence"])
        require(reference_svg.is_file()
                and hashlib.sha256(reference_svg.read_bytes()).hexdigest() == matched["reference_sha256"],
                f"MIT 图标参考证据 SHA-256 不符：{path}")
    return evidence


def validate_format(data: bytes, row: dict) -> None:
    path = row["path"]
    suffix = Path(path).suffix
    require(suffix in MIMES and row.get("mime") == MIMES[suffix], f"MIME 与扩展名不符：{path}")
    width, height = row.get("width"), row.get("height")
    if suffix == ".svg":
        try:
            xml = ET.fromstring(data)
            svg_width = float(xml.attrib["width"])
            svg_height = float(xml.attrib["height"])
        except (ET.ParseError, KeyError, ValueError) as error:
            raise ValueError(f"SVG 格式或尺寸无效：{path}: {error}") from error
        require(xml.tag == "{http://www.w3.org/2000/svg}svg", f"SVG 根元素无效：{path}")
        require(isinstance(width, (int, float)) and isinstance(height, (int, float))
                and width > 0 and height > 0 and svg_width == width and svg_height == height,
                f"SVG 声明尺寸与清单不一致：{path}")
    elif suffix == ".png":
        require(len(data) >= 33 and data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR",
                f"PNG 文件头无效：{path}")
        require(struct.unpack_from(">II", data, 16) == (width, height)
                and width > 0 and height > 0, f"PNG 尺寸与清单不一致：{path}")
    elif suffix == ".ico":
        require(len(data) >= 6 and data[:4] == b"\x00\x00\x01\x00", f"ICO 文件头无效：{path}")
        count = struct.unpack_from("<H", data, 4)[0]
        require(count == 3 and len(data) >= 6 + 16 * count, "ICO 必须包含 16/32/48 三帧")
        sizes = []
        end = 6 + 16 * count
        for index in range(count):
            w, h, colors, reserved, planes, bits, length, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16 * index)
            w, h = w or 256, h or 256
            require(w == h and planes == 1 and bits == 32 and colors == reserved == 0
                    and offset == end and length > 0 and offset + length <= len(data), "ICO 目录/帧范围无效")
            sizes.append(w)
            end = offset + length
        require(sizes == [16, 32, 48] and end == len(data) and width == height == 48,
                "ICO 尺寸或尾随数据与清单不符")
    else:
        require(width is None and height is None, f"非图像资产不得虚构尺寸：{path}")
        require(isinstance(row.get("dimensions_note"), str) and bool(row["dimensions_note"].strip()),
                f"null 尺寸缺少原因：{path}")
    if suffix in {".css", ".svg"}:
        data.decode("utf-8")
        require(data.endswith(b"\n") and b"\r" not in data, f"文本资产必须使用 LF 且保留末尾换行：{path}")
    if suffix == ".woff2":
        require(len(data) >= 48 and data[:4] == b"wOF2", f"字体不是 WOFF2：{path}")
        require(struct.unpack_from(">I", data, 8)[0] == len(data), f"WOFF2 头部文件长度不符：{path}")
        require(row["license"] == "OFL-1.1", f"字体不得使用项目许可证替代 OFL：{path}")


def build_manifest(root: Path) -> dict:
    sources = source_files(root)
    require(bool(sources), "没有找到 A00 静态源资产")
    metadata = load_metadata(root, sources)
    assets = []
    logical_names = set()
    for path in sorted(sources):
        row = metadata[path]
        for key in ("logical_name", "accessible_name", "source", "theme"):
            require(isinstance(row.get(key), str) and bool(row[key].strip()), f"{path} 缺少 {key}")
        require(row["logical_name"] not in logical_names, f"逻辑名重复：{row['logical_name']}")
        logical_names.add(row["logical_name"])
        data = local_path(root, path).read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if "sha256" in row:
            require(row["sha256"] == digest, f"资产源文件 SHA-256 与权利元数据不一致：{path}")
        evidence = validate_evidence(root, row)
        validate_format(data, row)
        item = {
            "logical_name": row["logical_name"],
            "path": path,
            "sha256": digest,
            "bytes": len(data),
            "mime": row["mime"],
            "width": row["width"],
            "height": row["height"],
            "theme": row["theme"],
            "accessible_name": row["accessible_name"],
            "source": row["source"],
            "license": row["license"],
            "redistributable": row["redistributable"],
            "evidence": evidence,
            "content_hash_filename": digest + Path(path).suffix,
            "content_hash_filename_status": "planned-a01-output-name; not built by A00",
        }
        for key in ("dimensions_note", "source_url", "font_version", "token_reference",
                    "local_versioning_decision", "public_release", "sizes", "modification"):
            if key in row:
                item[key] = row[key]
        assets.append(item)
    return {
        "schema_version": 1,
        "work_package": "WP-MVP-A00",
        "scope": "本清单只登记静态源码资产；不是生产运行时 manifest。",
        "generator": "python3 scripts/a00-assets.py --write",
        "verify": "python3 scripts/a00-assets.py",
        "content_hash_filename_contract": "content_hash_filename 仅记录未来 WP-MVP-A01 的建议输出名；A00 未运行 Node/esbuild、未生成 dist 或生产 manifest，也不声明指纹 URL 已可达。",
        "source_scan_exclusions": ["apps/web/static/dist/", ".gitkeep"],
        "unconfirmed_assets": (RIGHTS / "reference-brand-assets.json").as_posix(),
        "assets": assets,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="生成当前源码资产清单；不构建 dist")
    options = parser.parse_args()
    try:
        expected = build_manifest(ROOT)
        path = ROOT / MANIFEST
        if options.write:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(expected, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            print(f"已写入 {MANIFEST}（{len(expected['assets'])} 项源码资产；未构建 dist）")
        else:
            require(path.is_file(), f"源码资产清单缺失；先执行 python3 scripts/a00-assets.py --write：{MANIFEST}")
            actual = read_json(path)
            require(actual == expected, "资产清单与当前源码不一致；检查变更和权利后运行 --write 更新。")
            print(f"A00 资产检查通过：{len(expected['assets'])} 项，源码/清单双向完整、SHA-256、MIME、尺寸、实际许可状态与证据完整；未知许可仅按 DEC-045 本机版本化。")
        return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"A00 资产检查失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
