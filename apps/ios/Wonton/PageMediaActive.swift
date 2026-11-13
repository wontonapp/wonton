import WontonClient

/// 插件网页在放媒体：核心据此在 App 退到后台时照常探测、自动重连（ADR-0033）。
/// 整页和面板里的同一页各一份会话，谁在放以各会话的上报为准；都停了才解除，
/// 换曲的一瞬由核心的宽限兜住。
@MainActor
final class PageMediaActive {
    static let shared = PageMediaActive()

    private var playing: Set<ObjectIdentifier> = []
    private var reported = false

    /// 一个会话开始或停止放媒体；会话释放时也以 playing: false 走这里。
    func set(_ session: AnyObject, playing: Bool, core: ClientCore) {
        let id = ObjectIdentifier(session)
        if playing { self.playing.insert(id) } else { self.playing.remove(id) }
        let active = !self.playing.isEmpty
        guard active != reported else { return }
        reported = active
        core.pageMediaActive(active: active)
        #if DEBUG
        // 走查的证据：放媒体时上报了核心，后台才照常探测、自动重连。
        NSLog("Wonton page media active: %@", active ? "true" : "false")
        #endif
    }
}
