import Observation
import WontonClient

/// 主界面的三个页面。
enum AppTab {
    case chat
    case apps
    case settings
}

/// 从应用页打开的私聊：压在那一页的导航上，返回就回到那里。
struct VisitingRoom {
    let tab: AppTab
    let roomId: String
}

enum ChatRoute: Hashable {
    case room(String)
    case chatBackup
    case startChat
}

@MainActor
@Observable
final class ChatModel {
    private(set) var viewModel: ChatViewModel
    var rejection: Problem?
    /// 插件网页交给主人的草稿；只在聊天输入框出现，不自动发送。
    var pendingDraft: String?
    var drafts: [String: String] = [:]
    /// 聊天页打开的页面。
    var path: [ChatRoute] = []
    var visiting: VisitingRoom?
    /// 刚发起了和这个用户的私聊，房间出现后打开。
    private var awaitedPeer: String?
    @ObservationIgnored private var awaitedFrom = AppTab.chat
    /// 发起私聊在后台失败了，原因显示在聊天页，主界面要切过去。
    var openingFailed = false

    let core: ClientCore
    @ObservationIgnored private(set) lazy var files = FilesModel(chat: self)

    init(core: ClientCore) {
        self.core = core
        viewModel = core.chat()
    }

    /// 在主界面的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await _ in core.subscribeChat().updates() {
            // 系统 Intent 可在订阅消费前改状态，不能用队列中的旧快照覆盖它。
            viewModel = core.chat()
            openAwaitedChat()
        }
    }

    /// 发起了私聊，房间还没出现。
    var isOpeningChat: Bool { awaitedPeer != nil }

    func receiveDraft(_ text: String) {
        if case let .room(roomId) = path.last,
           viewModel.rooms.first(where: { $0.roomId == roomId })?.isReadOnly == false {
            appendDraft(text, to: roomId)
        } else {
            pendingDraft = [pendingDraft, text].compactMap { $0 }.joined(separator: "\n")
            path = []
        }
    }

    func acceptPendingDraft(in roomId: String) {
        guard let text = pendingDraft else { return }
        pendingDraft = nil
        appendDraft(text, to: roomId)
    }

    func appendDraft(_ text: String, to roomId: String) {
        let existing = drafts[roomId, default: ""]
        drafts[roomId] = existing.isEmpty ? text : existing + "\n" + text
    }

    /// `tab` 这一页正在显示的房间。
    func shownRoom(in tab: AppTab) -> String? {
        if let visiting, visiting.tab == tab { return visiting.roomId }
        if tab == .chat, case let .room(roomId) = path.last { return roomId }
        return nil
    }

    func title(of roomId: String) -> String {
        viewModel.rooms.first { $0.roomId == roomId }?.name ?? roomId
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }

    func resetNavigation() {
        awaitedPeer = nil
        openingFailed = false
        visiting = nil
        path = []
    }

    func startNewChat() {
        resetNavigation()
        path = [.startChat]
    }

    /// 从 `origin` 这一页打开和 `peer` 的私聊：已经加入的直接打开；还没有就发起，房间出现后打开，这期间留在原页；
    /// 默认保留邀请供主人接受；系统打开默认 Agent 时接受邀请并等加入。返回主界面要不要切到聊天页。
    @discardableResult
    func openDirectChat(with peer: String, from origin: AppTab = .chat, acceptingInvitation: Bool = false) -> Bool {
        awaitedPeer = nil
        path = []
        visiting = nil
        let userId: String
        do {
            userId = try core.resolveUserId(input: peer)
        } catch {
            rejection = error.problem
            return true
        }
        if acceptingInvitation { viewModel = core.chat() }
        let rooms = viewModel.rooms.filter { $0.directUserId == userId }
        if let joined = rooms.first(where: { $0.membership == .joined }) {
            show(joined.roomId, from: origin)
            return origin == .chat
        }
        let invitation = rooms.first { $0.membership == .invited }
        guard rooms.isEmpty || (acceptingInvitation && invitation != nil) else { return true }
        do {
            if acceptingInvitation { try core.dispatch(action: .dismissChatProblem) }
            if let invitation, acceptingInvitation {
                try core.dispatch(action: .joinRoom(roomId: invitation.roomId))
            } else {
                try core.dispatch(action: .startDirectChat(userId: userId))
            }
            awaitedPeer = userId
            awaitedFrom = origin
            if acceptingInvitation {
                viewModel = core.chat()
                openAwaitedChat()
            }
            return origin == .chat
        } catch {
            rejection = error.problem
            return true
        }
    }

    /// AskAgent 已在核心里发起或加入房间；这里只等订阅里的房间出现，避免重复建房。
    func openAskedAgentChat(with peer: String) {
        path = []
        visiting = nil
        awaitedPeer = peer
        awaitedFrom = .chat
        viewModel = core.chat()
        openAwaitedChat()
    }

    private func show(_ roomId: String, from origin: AppTab) {
        if origin == .chat {
            path = [.room(roomId)]
        } else {
            visiting = VisitingRoom(tab: origin, roomId: roomId)
        }
    }

    private func openAwaitedChat() {
        guard let peer = awaitedPeer else { return }
        if let room = viewModel.rooms.first(where: { $0.directUserId == peer && $0.membership == .joined }) {
            awaitedPeer = nil
            show(room.roomId, from: awaitedFrom)
        } else if viewModel.problem != nil {
            awaitedPeer = nil
            openingFailed = true
        }
    }
}
