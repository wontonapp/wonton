import Observation
import WontonClient

@MainActor
@Observable
final class ChatDraftStore {
    private var server: String?
    var bodies: [String: String] = [:]
    private(set) var replies: [String: ReplyQuote] = [:]
    private var knownReplies: Set<String> = []

    func selectServer(_ server: String?) {
        guard self.server != server else { return }
        self.server = server
        bodies.removeAll()
        replies.removeAll()
        knownReplies.removeAll()
    }

    func restore(_ stored: ComposerDraft?, in roomId: String) {
        if bodies[roomId] == nil { bodies[roomId] = stored?.body ?? "" }
        // 空回复可能已经被另一个窗口读取或取消，不能再用迟到的读取覆盖。
        if knownReplies.insert(roomId).inserted { replies[roomId] = stored?.reply }
    }

    func setReply(_ reply: ReplyQuote?, in roomId: String) {
        knownReplies.insert(roomId)
        replies[roomId] = reply
    }
}
