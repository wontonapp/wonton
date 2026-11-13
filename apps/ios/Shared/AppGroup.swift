import Foundation

/// App 和分享扩展共用的 App Group（ADR-0097）。组名由 project.yml 按 bundle ID 写进两边的 Info.plist（`WontonAppGroup`），
/// 真机换团队签名时跟着变，代码里不写死。
enum AppGroup {
    enum Failure: Error, CustomStringConvertible {
        /// Info.plist 里没有组名：工程不是按 project.yml 生成的。
        case missingIdentifier
        /// 系统不给这个组的容器：签名里没带 App Group 能力，或组名和描述文件对不上。
        case containerUnavailable(String)

        var description: String {
            switch self {
            case .missingIdentifier: "WontonAppGroup is missing from Info.plist"
            case let .containerUnavailable(group): "App Group container \(group) is unavailable"
            }
        }
    }

    /// 核心的数据目录：组容器里的 `wonton/`。扩展把分享写进这里的 `share-inbox/`。
    static func dataDirectory() throws(Failure) -> URL {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "WontonAppGroup") as? String, !group.isEmpty else {
            throw .missingIdentifier
        }
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else {
            throw .containerUnavailable(group)
        }
        return container.appending(path: "wonton", directoryHint: .isDirectory)
    }
}
