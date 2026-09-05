#!/usr/bin/env python3
"""Scan A00 asset filenames and decoded text metadata; this is not pixel OCR.

Reproduce the isolated development environment (not an A01 build dependency):
  python3 -m venv .local/a00-redo/asset-scan-venv
  .local/a00-redo/asset-scan-venv/bin/python -m pip install \
    'fonttools[woff]==4.64.0' 'brotli==1.2.0' 'zopfli==0.4.3'
  .local/a00-redo/asset-scan-venv/bin/python scripts/a00-asset-text-scan.py
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import platform
import struct
import sys
import unicodedata
import xml.etree.ElementTree as ET
import zlib


ROOT = Path(__file__).resolve().parents[1]
DOC = Path("docs/visual/a00")
PINNED = {"fonttools": "4.64.0", "Brotli": "1.2.0", "zopfli": "0.4.3"}
MAX_TEXT_BYTES = 16 * 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def normalize(text: str) -> str:
    return unicodedata.normalize("NFKC", text).casefold()


def dependencies():
    try:
        from fontTools.ttLib import TTFont
        import brotli  # noqa: F401: WOFF2 decompression is an explicit requirement.
        versions = {name: importlib.metadata.version(name) for name in PINNED}
    except (ImportError, importlib.metadata.PackageNotFoundError) as error:
        raise ValueError("缺少 fontTools/Brotli 开发扫描依赖；请按脚本顶部固定版本命令创建 .local 独立 venv，不得跳过字体元数据扫描。") from error
    require(versions == PINNED, f"扫描依赖版本与固定版本不符：{versions}；请使用脚本顶部安装命令。")
    return TTFont, versions


def xml_fields(data: bytes) -> list[tuple[str, str]]:
    decoded = data.decode("utf-8")
    require("<!DOCTYPE" not in decoded.upper(), "不接受带 DTD 的 SVG/XML 元数据")
    parser = ET.XMLParser(target=ET.TreeBuilder(insert_comments=True, insert_pis=True))
    root = ET.fromstring(data, parser=parser)
    fields = [("xml.raw", decoded)]
    for index, element in enumerate(root.iter()):
        if element.text:
            fields.append((f"xml.node[{index}].text", element.text))
        if element.tail:
            fields.append((f"xml.node[{index}].tail", element.tail))
        for key, value in element.attrib.items():
            fields.append((f"xml.node[{index}].attribute.{key}", value))
    return fields


def font_fields(path: Path, TTFont) -> tuple[list[tuple[str, str]], dict]:
    fields = []
    with TTFont(path, lazy=False, checkChecksums=2, ignoreDecompileErrors=False) as font:
        require(font.flavor == "woff2", "预期 WOFF2 字体")
        require("name" in font and bool(font["name"].names), "字体缺少可扫描 name 表")
        names = []
        for index, record in enumerate(font["name"].names):
            text = record.toUnicode(errors="strict")
            field = f"name[{index}].id{record.nameID}.platform{record.platformID}.encoding{record.platEncID}.language{record.langID}"
            fields.append((field, text))
            names.append({"name_id": record.nameID, "platform_id": record.platformID,
                          "encoding_id": record.platEncID, "language_id": record.langID,
                          "decoded_characters": len(text)})
        meta_present = "meta" in font
        if meta_present:
            for tag, value in font["meta"].data.items():
                text = value if isinstance(value, str) else value.decode("utf-8")
                fields.append((f"opentype.meta.{tag}", text))
        woff_metadata = font.flavorData.metaData
        if woff_metadata:
            fields.extend(("woff.metadata." + name, text) for name, text in xml_fields(woff_metadata))
        private_data = font.flavorData.privData
        if private_data:
            # Unstructured binary data cannot silently count as inspected text.
            fields.append(("woff.private_data", private_data.decode("utf-8")))
        details = {"name_records": names, "name_record_count": len(names),
                   "opentype_meta_present": meta_present,
                   "woff_xml_metadata_present": bool(woff_metadata),
                   "woff_private_data_present": bool(private_data)}
    return fields, details


def inflate_text(data: bytes) -> bytes:
    stream = zlib.decompressobj()
    result = stream.decompress(data, MAX_TEXT_BYTES + 1)
    require(len(result) <= MAX_TEXT_BYTES and stream.eof
            and not stream.unused_data and not stream.unconsumed_tail,
            "PNG 文本块压缩数据无效或超过 16 MiB 扫描上限")
    return result


def icc_fields(data: bytes) -> list[tuple[str, str]]:
    """Decode the text tags in the PNG profiles used by this capture pipeline.

    Unknown tag types fail explicitly; binary color parameters are not text.
    """
    require(len(data) >= 132 and struct.unpack_from(">I", data)[0] == len(data)
            and data[36:40] == b"acsp", "PNG ICC 头或声明长度无效")
    count = struct.unpack_from(">I", data, 128)[0]
    table_end = 132 + count * 12
    require(count > 0 and table_end <= len(data), "PNG ICC 标签目录截断")
    fields = []
    ranges = set()
    for index in range(count):
        signature, offset, size = struct.unpack_from(">4sII", data, 132 + index * 12)
        require(offset >= table_end and offset % 4 == 0 and size >= 8
                and offset + size <= len(data), "PNG ICC 标签范围无效")
        block = data[offset:offset + size]
        kind = block[:4]
        require(block[4:8] == bytes(4), "PNG ICC 标签保留字段非零")
        prefix = f"icc.tag[{index}]"
        fields.append((prefix + ".signature", signature.decode("ascii")))
        if kind == b"mluc":
            require(size >= 16, "PNG ICC mluc 头截断")
            records, record_size = struct.unpack_from(">II", block, 8)
            records_end = 16 + records * 12
            require(record_size == 12 and records_end <= size, "PNG ICC mluc 目录无效")
            text_ranges = set()
            for record in range(records):
                locale, length, start = struct.unpack_from(">4sII", block, 16 + record * 12)
                require(length % 2 == 0 and start >= records_end and start + length <= size,
                        "PNG ICC mluc 文本范围无效")
                fields.extend([(f"{prefix}.mluc[{record}].locale", locale.decode("ascii")),
                               (f"{prefix}.mluc[{record}].text", block[start:start + length].decode("utf-16-be"))])
                text_ranges.add((start, start + length))
            cursor = records_end
            for start, end in sorted(text_ranges):
                require(start >= cursor and not any(block[cursor:start]),
                        "PNG ICC mluc 文本重叠或含未解析数据")
                cursor = end
            require(not any(block[cursor:]), "PNG ICC mluc 存在未解析尾随数据")
        elif kind == b"text":
            raw = block[8:]
            text, _, padding = raw.partition(b"\0")
            require(not any(padding), "PNG ICC text 存在未解析尾随数据")
            fields.append((prefix + ".text", text.decode("ascii")))
        elif kind == b"XYZ ":
            require(size >= 20 and (size - 8) % 12 == 0, "PNG ICC XYZ 数值长度无效")
        elif kind == b"para":
            require(size >= 12, "PNG ICC 参数曲线头截断")
            function, reserved = struct.unpack_from(">HH", block, 8)
            parameters = {0: 1, 1: 3, 2: 4, 3: 5, 4: 7}
            require(reserved == 0 and function in parameters
                    and size == 12 + parameters[function] * 4, "PNG ICC 参数曲线无效")
        else:
            raise ValueError(f"PNG ICC 标签类型尚未实现扫描：{kind!r}")
        ranges.add((offset, offset + size))
    cursor = table_end
    for start, end in sorted(ranges):
        require(start == (cursor + 3) // 4 * 4 and not any(data[cursor:start]),
                "PNG ICC 标签重叠或含未解析间隙")
        cursor = end
    require(len(data) in (cursor, (cursor + 3) // 4 * 4) and not any(data[cursor:]),
            "PNG ICC 存在未解析尾随数据")
    return fields


def png_fields(path: Path) -> tuple[list[tuple[str, str]], dict]:
    data = path.read_bytes()
    require(data[:8] == b"\x89PNG\r\n\x1a\n", "文件不是 PNG")
    offset = 8
    fields = []
    chunks = []
    counts = {"tEXt": 0, "zTXt": 0, "iTXt": 0}
    icc_profiles = 0
    # Numeric color/geometry chunks contain no text. Unknown ancillary chunks
    # (including eXIf) must not silently pass as fully scanned metadata.
    nontext_chunks = {"IHDR", "PLTE", "IDAT", "IEND", "cHRM", "gAMA", "sBIT",
                      "sRGB", "bKGD", "hIST", "tRNS", "pHYs", "tIME", "cICP", "mDCv", "cLLi"}
    while offset < len(data):
        require(offset + 12 <= len(data), "PNG 块头截断")
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        end = offset + 12 + length
        require(end <= len(data), "PNG 块数据截断")
        payload = data[offset + 8:offset + 8 + length]
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        require(zlib.crc32(kind + payload) & 0xFFFFFFFF == crc, "PNG 块 CRC 不符")
        kind_name = kind.decode("ascii")
        require(kind_name in nontext_chunks or kind_name in counts or kind_name == "iCCP",
                f"PNG 块尚未实现元数据扫描：{kind_name}")
        chunks.append(kind_name)
        if kind == b"IHDR":
            require(len(chunks) == 1 and length == 13, "PNG IHDR 位置或长度无效")
        elif kind == b"IEND":
            require(length == 0, "PNG IEND 必须为空")
        elif kind == b"iCCP":
            require(icc_profiles == 0, "PNG 含重复 ICC profile")
            name, compressed = payload.split(b"\0", 1)
            require(1 <= len(name) <= 79 and compressed and compressed[0] == 0,
                    "PNG ICC 名称或压缩方式无效")
            fields.append(("png.iCCP.profile_name", name.decode("latin-1")))
            fields.extend(("png.iCCP." + field, text)
                          for field, text in icc_fields(inflate_text(compressed[1:])))
            icc_profiles += 1
        if kind_name in counts:
            index = counts[kind_name]
            counts[kind_name] += 1
            keyword, rest = payload.split(b"\0", 1)
            require(1 <= len(keyword) <= 79, "PNG 文本块 keyword 长度无效")
            prefix = f"png.{kind_name}[{index}]"
            fields.append((prefix + ".keyword", keyword.decode("latin-1")))
            if kind == b"tEXt":
                text = rest.decode("latin-1")
            elif kind == b"zTXt":
                require(bool(rest) and rest[0] == 0, "PNG zTXt 压缩方式无效")
                text = inflate_text(rest[1:]).decode("latin-1")
            else:
                require(len(rest) >= 2 and rest[0] in (0, 1) and rest[1] == 0,
                        "PNG iTXt 压缩标记或方式无效")
                compressed = rest[0] == 1
                language, translated_and_text = rest[2:].split(b"\0", 1)
                translated, encoded_text = translated_and_text.split(b"\0", 1)
                fields.extend([(prefix + ".language", language.decode("ascii")),
                               (prefix + ".translated_keyword", translated.decode("utf-8"))])
                text = (inflate_text(encoded_text) if compressed else encoded_text).decode("utf-8")
            fields.append((prefix + ".text", text))
            if text.lstrip().startswith("<"):
                fields.extend((prefix + "." + name, value)
                              for name, value in xml_fields(text.encode("utf-8")))
        offset = end
        if kind == b"IEND":
            require(offset == len(data), "PNG IEND 后有未解析数据")
            break
    require(bool(chunks) and chunks[0] == "IHDR" and chunks[-1] == "IEND", "PNG 缺少 IHDR/IEND")
    return fields, {"chunks": chunks, "text_chunk_counts": counts,
                    "text_chunk_count": sum(counts.values()), "icc_profiles_scanned": icc_profiles,
                    "all_chunk_crcs_valid": True}


def ico_fields(path: Path) -> tuple[list[tuple[str, str]], dict]:
    """Validate every byte range of the A00 32-bit BI_RGB DIB icon container.

    ICONDIR/ICONDIRENTRY/BITMAPINFOHEADER have no textual metadata fields.
    XOR color pixels and AND masks are opaque image data for separate OCR.
    Other DIB headers, palettes, compression and embedded PNG fail closed.
    """
    data = path.read_bytes()
    require(len(data) >= 6, "ICO 文件头截断")
    reserved, image_type, count = struct.unpack_from("<HHH", data)
    require(reserved == 0 and image_type == 1 and count > 0, "ICO 容器头无效")
    directory_end = 6 + count * 16
    require(directory_end <= len(data), "ICO 图像目录截断")
    frames = []
    ranges = []
    for index in range(count):
        width, height, colors, reserved, planes, depth, size, offset = struct.unpack_from(
            "<BBBBHHII", data, 6 + index * 16)
        width, height = width or 256, height or 256
        require(colors == reserved == 0 and planes == 1 and depth == 32,
                "ICO 只支持已交付的无调色板 32 位图像目录")
        require(offset >= directory_end and size >= 40 and offset + size <= len(data),
                "ICO 帧偏移或长度越界")
        header = struct.unpack_from("<IiiHHIIiiII", data, offset)
        (header_size, dib_width, dib_height, dib_planes, dib_depth, compression,
         image_size, x_ppm, y_ppm, colors_used, colors_important) = header
        require(header_size == 40 and compression == 0 and dib_planes == 1 and dib_depth == 32
                and colors_used == colors_important == 0,
                "ICO DIB 必须是无调色板的 40 字节 BI_RGB/32 位头")
        require(dib_width == width and dib_height == height * 2,
                "ICO 目录与 DIB 宽高不一致")
        xor_stride = ((width * depth + 31) // 32) * 4
        and_stride = ((width + 31) // 32) * 4
        xor_bytes, and_bytes = xor_stride * height, and_stride * height
        require(image_size in (0, xor_bytes, xor_bytes + and_bytes), "ICO DIB sizeImage 与像素范围不符")
        require(size == header_size + xor_bytes + and_bytes,
                "ICO DIB 像素或掩码截断，或含未解释的尾随数据")
        ranges.append((offset, offset + size))
        frames.append({"index": index, "width": width, "height": height,
                       "offset": offset, "bytes": size, "sha256": hashlib.sha256(data[offset:offset + size]).hexdigest(),
                       "dib_header_bytes": header_size, "bits_per_pixel": depth, "compression": "BI_RGB",
                       "declared_image_bytes": image_size, "pixels_per_meter": [x_ppm, y_ppm],
                       "xor_range": [offset + header_size, offset + header_size + xor_bytes],
                       "and_range": [offset + header_size + xor_bytes, offset + size],
                       "text_metadata_fields": 0})
    cursor = directory_end
    for start, end in sorted(ranges):
        require(start == cursor, "ICO 帧重叠或含未解释的间隙")
        cursor = end
    require(cursor == len(data), "ICO 含未解释的尾随数据")
    return [], {"format_scope": "ICO with 40-byte BITMAPINFOHEADER, uncompressed 32-bit BI_RGB DIB frames",
                "frame_count": count, "frames": frames, "all_byte_ranges_accounted_for": True,
                "text_metadata_fields": 0,
                "metadata_note": "Supported ICO/DIB headers have numeric fields only; pixel and AND-mask bytes are not text and require separate image review/OCR."}


def scan(root: Path) -> dict:
    wordlist_path = root / DOC / "old-names.json"
    wordlist_bytes = wordlist_path.read_bytes()
    wordlist = json.loads(wordlist_bytes)
    terms = sorted({normalize(term) for term in wordlist["terms"]})
    require(bool(terms) and all(terms), "旧称词表为空或包含空词")
    report = {"status": "fail", "scope": "Source asset filenames; decoded WOFF2 name/OpenType meta/WOFF XML metadata; SVG/XML text, attributes and comments; static and implementation PNG tEXt/zTXt/iTXt and ICC text metadata; static ICO/DIB container and every frame byte range. Raster pixels require separate OCR/manual review.",
              "wordlist": "docs/visual/a00/old-names.json",
              "wordlist_sha256": hashlib.sha256(wordlist_bytes).hexdigest(),
              "normalization": "Unicode NFKC + casefold, substring match",
              "normalized_terms": terms,
              "scanner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "tools": {"python": platform.python_version()},
              "reproduce": ["python3 -m venv .local/a00-redo/asset-scan-venv",
                            ".local/a00-redo/asset-scan-venv/bin/python -m pip install 'fonttools[woff]==4.64.0' 'brotli==1.2.0' 'zopfli==0.4.3'",
                            ".local/a00-redo/asset-scan-venv/bin/python scripts/a00-asset-text-scan.py"],
              "exclusions": ["apps/web/static/dist/ and .gitkeep sentinels", "Attributed reference evidence and license/source records", "Raster pixels: separately covered by OCR"],
              "files": [], "matches": [], "errors": []}
    try:
        TTFont, versions = dependencies()
        report["tools"].update(versions)
    except ValueError as error:
        report["errors"].append(str(error))
        return report
    static = root / "apps/web/static"
    sources = sorted(path for path in static.rglob("*") if path.is_file()
                     and path.relative_to(static).parts[0] != "dist" and path.name != ".gitkeep")
    implementation = root / DOC / "implementation"
    pngs = sorted(implementation.glob("*.png"))
    preview = json.loads((implementation / "manifest.json").read_text())
    expected_pngs = {capture["screenshot"]: capture["sha256"] for capture in preview["captures"]}
    require(set(expected_pngs) == {path.name for path in pngs}, "实现 PNG 与截图清单不是双向完整")
    require(bool(pngs) and any(path.suffix == ".woff2" for path in sources), "缺少必须扫描的 PNG 或 WOFF2")
    for path in sources + pngs:
        relative = path.relative_to(root).as_posix()
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        entry = {"path": relative, "sha256": digest, "filename_scanned": True}
        fields = [("filename", relative)]
        try:
            if path.suffix == ".woff2":
                extra, details = font_fields(path, TTFont)
                entry.update(kind="woff2", **details)
                fields.extend(extra)
            elif path.suffix in {".svg", ".xml"}:
                fields.extend(xml_fields(data))
                entry["kind"] = "svg/xml"
            elif path.suffix == ".png":
                if path in pngs:
                    require(digest == expected_pngs[path.name], "PNG SHA-256 与最终截图清单不一致")
                extra, details = png_fields(path)
                fields.extend(extra)
                entry.update(kind="png", asset_scope="implementation" if path in pngs else "static", **details)
            elif path.suffix == ".ico":
                extra, details = ico_fields(path)
                fields.extend(extra)
                entry.update(kind="ico", **details)
            else:
                require(path.suffix == ".css", f"未支持的资产类型，不能只扫描文件名：{path.suffix}")
                entry["kind"] = "filename-only; source text handled by a00-check.py"
            for field, text in fields:
                normalized = normalize(text)
                for term in terms:
                    if term in normalized:
                        report["matches"].append({"path": relative, "field": field, "term": term})
            entry["decoded_text_fields_scanned"] = len(fields) - 1
            entry["decoded_metadata_characters"] = sum(len(text) for _, text in fields[1:])
        except Exception as error:
            report["errors"].append(f"{relative}: {type(error).__name__}: {error}")
        report["files"].append(entry)
    report["counts"] = {"filenames": len(report["files"]),
                        "woff2": sum(row.get("kind") == "woff2" for row in report["files"]),
                        "svg_xml": sum(row.get("kind") == "svg/xml" for row in report["files"]),
                        "implementation_png": len(pngs),
                        "static_png": sum(row.get("kind") == "png" and row.get("asset_scope") == "static" for row in report["files"]),
                        "ico": sum(row.get("kind") == "ico" for row in report["files"]),
                        "ico_frames": sum(row.get("frame_count", 0) for row in report["files"]),
                        "font_name_records": sum(row.get("name_record_count", 0) for row in report["files"]),
                        "png_text_chunks": sum(row.get("text_chunk_count", 0) for row in report["files"]),
                        "png_icc_profiles": sum(row.get("icc_profiles_scanned", 0) for row in report["files"])}
    report["status"] = "pass" if not report["errors"] and not report["matches"] else "fail"
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--report", default="docs/visual/a00/asset-text-scan.json",
                        help="报告路径；默认写入版本化 A00 证据目录")
    args = parser.parse_args()
    try:
        report = scan(ROOT)
        output = ROOT / args.report
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({key: report.get(key) for key in ("status", "counts", "matches", "errors")}, ensure_ascii=False, indent=2))
        return 0 if report["status"] == "pass" else 1
    except (OSError, ValueError, KeyError) as error:
        print(f"A00 资产文字扫描失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
