import Foundation
import OSLog
import SwiftUI

/// 快照的 JSON 投影，字段名归 `wonton-protocol::widget` 所有（ADR-0100）；这个目录特意不链接核心，按字段名自己解。
struct EntrySnapshot: Decodable, Sendable {
    let version: UInt32
    let updatedAtMs: UInt64
    let account: Account
    let unreadTotal: UInt64
    let rooms: [Room]
    let defaultAgent: Agent?
    let plugins: [Plugin]
    let light: Palette
    let dark: Palette

    enum Account: String, Decodable, Sendable {
        case signedIn = "signed_in"
        case notPaired = "not_paired"
        case signedOut = "signed_out"
        case switching
    }

    struct Room: Decodable, Sendable {
        let roomId: String
        let name: String
        let unreadCount: UInt64
        let preview: Preview?
    }

    struct Preview: Decodable, Sendable {
        let senderName: String?
        let isOwn: Bool
        let body: Body

        struct Body: Decodable, Sendable {
            let type: String
            let text: String?
            let title: String?
            let state: String?
            /// 未知事件的事件类型原文（如 m.sticker），预览里不翻译。
            var kind: String? = nil
        }
    }

    struct Agent: Decodable, Sendable {
        let name: String
        let activity: String?
        let unreadCount: UInt64
    }

    struct Plugin: Decodable, Sendable {
        let pluginId: String
        let title: String
    }

    struct Palette: Decodable, Sendable {
        let surface: RGB
        let textPrimary: RGB
        let textSecondary: RGB
        let accentFill: RGB
        /// 页面底色上的图标（ADR-0026）。
        let accentInk: RGB
        let accentInkOnFill: RGB
    }

    struct RGB: Decodable, Sendable {
        let red: UInt8
        let green: UInt8
        let blue: UInt8

        var color: Color {
            Color(.sRGB, red: Double(red) / 255, green: Double(green) / 255, blue: Double(blue) / 255, opacity: 1)
        }
    }

    var updatedAt: Date { Date(timeIntervalSince1970: Double(updatedAtMs) / 1_000) }

    static func decode(_ data: Data) throws -> EntrySnapshot {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let version = try decoder.decode(Version.self, from: data).version
        guard version == 1 else { throw ReadFailure.unsupportedVersion(version) }
        return try decoder.decode(Self.self, from: data)
    }

    private struct Version: Decodable { let version: UInt32 }

    enum ReadFailure: Error {
        case unsupportedVersion(UInt32)
    }
}

enum EntrySnapshotState: Sendable {
    case missing
    case unreadable(String)
    case ready(EntrySnapshot)

    var snapshot: EntrySnapshot? {
        if case let .ready(snapshot) = self { return snapshot }
        return nil
    }

    var signedIn: EntrySnapshot? {
        guard let snapshot, snapshot.account == .signedIn else { return nil }
        return snapshot
    }
}

enum EntrySnapshotStore {
    static func read() -> EntrySnapshotState {
        do {
            let url = try AppGroup.dataDirectory().appending(path: "widget/snapshot.json")
            return try read(at: url)
        } catch {
            return unreadable(error)
        }
    }

    static func read(at url: URL) throws -> EntrySnapshotState {
        do {
            return .ready(try EntrySnapshot.decode(Data(contentsOf: url)))
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return .missing
        } catch {
            return unreadable(error)
        }
    }

    private static func unreadable(_ error: any Error) -> EntrySnapshotState {
        let detail = String(describing: error)
        Logger(subsystem: Bundle.main.bundleIdentifier ?? "app.wonton", category: "WidgetSnapshot")
            .error("Cannot read widget snapshot: \(detail, privacy: .public)")
        return .unreadable(detail)
    }
}

/// 链接拼法照 ADR-0041/0100；扩展不链接核心，编解码在壳里各有一份，字段名与路径由协议的契约测试锁住。
enum EntryLink {
    static let chat = URL(string: "wonton://tab/chat")!
    static let agent = URL(string: "wonton://agent-chat")!
    static let apps = URL(string: "wonton://tab/apps")!

    /// 房间 ID 按百分号编码，`!` 和 `:` 原样留着（ADR-0100 决策 7，字符集同 wonton-protocol 的 ROOM_ID）。
    static func room(_ id: String) -> URL {
        let encoded = id.addingPercentEncoding(withAllowedCharacters: Self.roomCharacters) ?? id
        return URL(string: "wonton://room/\(encoded)")!
    }

    static func plugin(_ id: String) -> URL {
        URL(string: "wonton://tab/plugin/\(id)")!
    }

    private static let roomCharacters = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~!$&'()*+,;=:@")
}
