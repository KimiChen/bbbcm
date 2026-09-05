#!/usr/bin/env swift
// A00 development evidence: local macOS Vision OCR; stdout only, never writes images/results.
// Usage: swift scripts/a00-ocr.swift --terms words.txt [--reference-zones zones.json] image.png ...
// Words: one literal term per line (# comments), or {"terms":["term", ...]} / ["term", ...].
// Zones: [{"path":"docs/visual/a00/reference/image.png","rect":{"x":0,"y":0,
// "width":200,"height":60},"terms":["烧饼论坛"],"reason":"public reference header"}].
// Rectangles use top-left image pixels. Only the named reference path, word and box are allowed.

import CryptoKit
import Foundation
import ImageIO
import Vision

struct ScanError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

struct PixelRect: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double

    func contains(_ other: PixelRect) -> Bool {
        other.x >= x && other.y >= y && other.x + other.width <= x + width
            && other.y + other.height <= y + height
    }
}

struct ReferenceZone: Decodable {
    let path: String
    let rect: PixelRect
    let terms: [String]
    let reason: String
}

struct TermHit: Encodable {
    let term: String
    let disposition: String
    let reason: String?
}

struct Observation: Encodable {
    let text: String
    let confidence: Float
    let rect: PixelRect
    let hits: [TermHit]
}

struct ImageResult: Encodable {
    let path: String
    let sha256: String
    let width: Int
    let height: Int
    let observations: [Observation]
    let reviewHits: Int
    let allowedReferenceHits: Int
}

struct Report: Encodable {
    let schemaVersion = 1
    let engine = "macOS Vision VNRecognizeTextRequest accurate"
    let operatingSystem = ProcessInfo.processInfo.operatingSystemVersionString
    let languages = ["zh-Hans", "en-US"]
    let coordinateSystem = "top-left image pixels"
    let purpose = "A00 development OCR for manual review; not proof that sensitive text is absent"
    let wordlist: String
    let terms: [String]
    let referenceZones: String?
    let images: [ImageResult]
    let reviewHits: Int
    let allowedReferenceHits: Int
}

func absolutePath(_ path: String) -> String {
    URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath().path
}

func normalized(_ text: String) -> String {
    text.folding(options: [.caseInsensitive, .widthInsensitive], locale: Locale(identifier: "en_US_POSIX"))
        .filter { !$0.isWhitespace }
}

func readTerms(_ path: String) throws -> [String] {
    let data = try Data(contentsOf: URL(fileURLWithPath: path))
    let values: [String]
    if URL(fileURLWithPath: path).pathExtension.lowercased() == "json" {
        let object = try JSONSerialization.jsonObject(with: data)
        if let array = object as? [String] {
            values = array
        } else if let array = (object as? [String: Any])?["terms"] as? [String] {
            values = array
        } else {
            throw ScanError(message: "Wordlist JSON must be [String] or {\"terms\":[String]}.")
        }
    } else {
        guard let contents = String(data: data, encoding: .utf8) else {
            throw ScanError(message: "Wordlist must be UTF-8.")
        }
        values = contents.components(separatedBy: .newlines)
    }
    let terms = Array(Set(values.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty && !$0.hasPrefix("#") })).sorted()
    guard !terms.isEmpty else { throw ScanError(message: "Wordlist must contain at least one literal term.") }
    return terms
}

func scan(path: String, terms: [String], zones: [ReferenceZone]) throws -> ImageResult {
    let url = URL(fileURLWithPath: path)
    let data = try Data(contentsOf: url)
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
        CGImageSourceGetType(source) as String? == "public.png",
        let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { throw ScanError(message: "Input is not a readable PNG: \(path)") }
    let width = image.width
    let height = image.height
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["zh-Hans", "en-US"]
    request.usesLanguageCorrection = false
    try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
    let applicableZones = zones.filter { absolutePath($0.path) == absolutePath(path) }
    var observations: [Observation] = []
    for result in request.results ?? [] {
        guard let candidate = result.topCandidates(1).first else { continue }
        let box = result.boundingBox
        let rect = PixelRect(x: box.minX * Double(width), y: (1 - box.maxY) * Double(height),
            width: box.width * Double(width), height: box.height * Double(height))
        let content = normalized(candidate.string)
        let hits = terms.filter { content.contains(normalized($0)) }.map { term in
            let zone = applicableZones.first { $0.terms.contains(term) && $0.rect.contains(rect) }
            return TermHit(term: term,
                disposition: zone == nil ? "manual-review-required" : "allowed-reference-brand-position",
                reason: zone?.reason)
        }
        observations.append(Observation(text: candidate.string, confidence: candidate.confidence,
            rect: rect, hits: hits))
    }
    observations.sort { left, right in
        left.rect.y == right.rect.y ? left.rect.x < right.rect.x : left.rect.y < right.rect.y
    }
    let allHits = observations.flatMap(\.hits)
    return ImageResult(path: path, sha256: SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined(),
        width: width, height: height, observations: observations,
        reviewHits: allHits.filter { $0.disposition == "manual-review-required" }.count,
        allowedReferenceHits: allHits.filter { $0.disposition == "allowed-reference-brand-position" }.count)
}

func run() throws {
    var args = Array(CommandLine.arguments.dropFirst())
    var wordlist: String?
    var zonesPath: String?
    var paths: [String] = []
    while !args.isEmpty {
        let argument = args.removeFirst()
        if argument == "--terms" || argument == "--reference-zones" {
            guard !args.isEmpty else { throw ScanError(message: "Missing path after \(argument).") }
            let value = args.removeFirst()
            if argument == "--terms" { wordlist = value } else { zonesPath = value }
        } else if argument.hasPrefix("--") {
            throw ScanError(message: "Unknown option: \(argument)")
        } else {
            paths.append(argument)
        }
    }
    guard let wordlist, !paths.isEmpty else {
        throw ScanError(message: "Usage: swift scripts/a00-ocr.swift --terms words.txt [--reference-zones zones.json] image.png ...")
    }
    let terms = try readTerms(wordlist)
    let zones: [ReferenceZone]
    if let zonesPath {
        zones = try JSONDecoder().decode([ReferenceZone].self, from: Data(contentsOf: URL(fileURLWithPath: zonesPath)))
        let referenceRoot = absolutePath("docs/visual/a00/reference") + "/"
        for zone in zones {
            guard absolutePath(zone.path).hasPrefix(referenceRoot),
                !zone.reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                !zone.terms.isEmpty, zone.terms.allSatisfy(terms.contains),
                zone.rect.x >= 0, zone.rect.y >= 0, zone.rect.width > 0, zone.rect.height > 0
            else { throw ScanError(message: "Reference zone must name an A00 reference image, listed terms, a positive box and a reason: \(zone.path)") }
        }
    } else { zones = [] }
    let images = try paths.map { try scan(path: $0, terms: terms, zones: zones) }
    let report = Report(wordlist: wordlist, terms: terms, referenceZones: zonesPath, images: images,
        reviewHits: images.reduce(0) { $0 + $1.reviewHits },
        allowedReferenceHits: images.reduce(0) { $0 + $1.allowedReferenceHits })
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    let encoded = try encoder.encode(report)
    FileHandle.standardOutput.write(encoded)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

do {
    try run()
} catch {
    FileHandle.standardError.write(Data("a00-ocr: \(error.localizedDescription)\n".utf8))
    exit(1)
}
