#if DEBUG
import SwiftUI
import WontonClient

/// 影视页的临时入口：启动参数带 `wonton-video-preview` 时整屏是影视页（同 `PeersPreview` 的做法）。
/// 插件页按 `NativePage::Video` 打开影视页之后删掉。
enum VideoPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-video-preview")
    }

    @MainActor static func root(core: ClientCore) -> some View {
        VideoRoot(core: core)
    }
}
#endif
