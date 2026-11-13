import Foundation
import UniformTypeIdentifiers

/// 一次分享的收件目录 `<数据目录>/share-inbox/<令牌>/`（ADR-0097）。格式和校验在 `wonton-protocol` 的 `share_in` 模块，
/// 这里照那边契约测试里的样例写：文件、`manifest.json`，最后写 `.ready`。
@MainActor
struct ShareInbox {
    // 同 wonton-protocol 的 SHARE_INBOX_DIR、SHARE_MANIFEST_FILE、SHARE_READY_FILE、SHARE_MAX_ITEMS、
    // ShareInManifest::VERSION；扩展不链接核心。
    private static let inboxDir = "share-inbox"
    private static let manifestFile = "manifest.json"
    private static let readyFile = ".ready"
    private static let maxItems = 10
    private static let version = 1

    enum Failure: Error, CustomStringConvertible {
        case nothingShared
        case unreadable(String, String)

        var description: String {
            switch self {
            case .nothingShared: "nothing to share"
            case let .unreadable(type, reason): "\(type): \(reason)"
            }
        }
    }

    private let directory: URL

    init(token: String) throws {
        directory = try AppGroup.dataDirectory()
            .appending(path: Self.inboxDir, directoryHint: .isDirectory)
            .appending(path: token, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    func remove() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// 照片、视频、文件复制进来，文字和网页链接拼成一段。读不了任何一项都整次失败：不悄悄少发。
    func write(_ providers: [NSItemProvider]) async throws {
        var texts: [String] = []
        var items: [ShareManifestItem] = []
        for provider in providers {
            if let text = try await Self.text(from: provider) {
                texts.append(text)
                continue
            }
            let file = String(items.count)
            // 超过上限的只记进清单不复制：核心只读前 10 项，其余计数后在分享页提示丢了几项。
            if items.count < Self.maxItems {
                items.append(try await copy(provider, to: file))
            } else {
                items.append(ShareManifestItem(file: file, name: provider.suggestedName ?? file, mime: nil))
            }
        }
        let text = texts.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !items.isEmpty else { throw Failure.nothingShared }
        let manifest = ShareManifest(version: Self.version, text: text.isEmpty ? nil : text, items: items)
        try JSONEncoder().encode(manifest).write(to: directory.appending(path: Self.manifestFile))
        // 最后写：核心见到它才认领（没写完的目录被删掉）。
        try Data().write(to: directory.appending(path: Self.readyFile))
    }

    /// 网页链接和纯文字当文字；文件链接、照片、文件不算。
    private static func text(from provider: NSItemProvider) async throws -> String? {
        let types = provider.registeredTypeIdentifiers.compactMap(UTType.init)
        if types.contains(where: { $0.conforms(to: .fileURL) || $0.conforms(to: .image) || $0.conforms(to: .movie) }) {
            return nil
        }
        if types.contains(where: { $0.conforms(to: .url) }) { return try await loadString(provider, .url) }
        if types.contains(where: { $0.conforms(to: .plainText) }) { return try await loadString(provider, .plainText) }
        return nil
    }

    /// 在回调里就转成字符串：回调给的对象不是 Sendable，不带出回调。
    private static func loadString(_ provider: NSItemProvider, _ type: UTType) async throws -> String? {
        try await withCheckedThrowingContinuation { continuation in
            provider.loadItem(forTypeIdentifier: type.identifier) { item, error in
                if let error {
                    continuation.resume(throwing: Failure.unreadable(type.identifier, String(describing: error)))
                    return
                }
                switch item {
                case let url as URL: continuation.resume(returning: url.absoluteString)
                case let text as String: continuation.resume(returning: text)
                case let data as Data: continuation.resume(returning: String(data: data, encoding: .utf8))
                default: continuation.resume(returning: nil)
                }
            }
        }
    }

    /// `loadFileRepresentation` 给的临时文件在回调结束时就被删掉，所以在回调里同步复制。
    private func copy(_ provider: NSItemProvider, to file: String) async throws -> ShareManifestItem {
        let types = provider.registeredTypeIdentifiers.compactMap(UTType.init)
        guard let type = types.first(where: { $0.conforms(to: .image) || $0.conforms(to: .movie) })
            ?? types.first(where: { $0.conforms(to: .data) }) else {
            let registered = provider.registeredTypeIdentifiers.joined(separator: ",")
            throw Failure.unreadable(registered, "no file representation")
        }
        let target = directory.appending(path: file)
        let name: String = try await withCheckedThrowingContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                guard let url else {
                    let reason = error.map { String(describing: $0) } ?? "no file"
                    continuation.resume(throwing: Failure.unreadable(type.identifier, reason))
                    return
                }
                do {
                    try FileManager.default.copyItem(at: url, to: target)
                    continuation.resume(returning: url.lastPathComponent)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
        // 系统给的名字常常不带扩展名（相册的照片），补上临时文件的。
        let tempExtension = (name as NSString).pathExtension
        let suggested = provider.suggestedName.map { suggested in
            (suggested as NSString).pathExtension.isEmpty && !tempExtension.isEmpty
                ? "\(suggested).\(tempExtension)" : suggested
        }
        return ShareManifestItem(file: file, name: suggested ?? name, mime: type.preferredMIMEType)
    }
}

/// `manifest.json`，字段同 wonton-protocol 的 `ShareInManifest`。
private struct ShareManifest: Encodable {
    let version: Int
    let text: String?
    let items: [ShareManifestItem]

    enum CodingKeys: String, CodingKey {
        case version = "v"
        case text
        case items
    }
}

private struct ShareManifestItem: Encodable {
    let file: String
    let name: String
    let mime: String?
}
