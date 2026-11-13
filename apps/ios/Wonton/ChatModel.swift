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
    /// 每个房间里正在回复的消息；和草稿一起存，换房间、杀 App 再回来还在。
    var replies: [String: ReplyQuote] = [:]
    private(set) var replyFocusRevision = 0
    /// 聊天页打开的页面。
    var path: [ChatRoute] = []
    var visiting: VisitingRoom?
    /// 开着的旁聊：房间 ID → 旁聊根的事件 ID。一个房间同时最多一个（ADR-0093）。
    var sideChats: [String: String] = [:]
    /// 正在写第一句的房间：开新旁聊的面板开着。
    var composingSideChat: String?
    /// 新建聊天选的 AI 不支持旁聊、打开的是主对话的房间（ADR-0093 §7）。
    var noSideChatsRoom: String?
    /// 刚发起了和这个用户的私聊，房间出现后打开。
    private var awaitedPeer: String?
    /// 链接要打开的房间；还需要加入或等同步。
    private var awaitedRoom: String?
    @ObservationIgnored private var awaitedFrom = AppTab.chat
    /// 发起私聊在后台失败了，原因显示在聊天页，主界面要切过去。
    var openingFailed = false
    /// 主人在交接页点了“稍后”（ADR-0087）：先收起交接页，聊天列表顶上的提醒再打开。只在这次运行里记着。
    var keyHandoffDeferred = false

    let core: ClientCore
    @ObservationIgnored private(set) lazy var files = FilesModel(chat: self)

    init(core: ClientCore) {
        self.core = core
        viewModel = core.chat()
    }

    #if DEBUG
        /// 截图测试拿固定 ViewModel 画界面，不订阅核心推送。
        init(core: ClientCore, viewModel: ChatViewModel) {
            self.core = core
            self.viewModel = viewModel
        }
    #endif

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
            appendDraft(text, to: TimelineTarget(roomId: roomId, threadRoot: nil))
        } else {
            pendingDraft = [pendingDraft, text].compactMap { $0 }.joined(separator: "\n")
            path = []
        }
    }

    /// 草稿和回复按时间线分开存（ADR-0093）：主时间线是房间 ID，旁聊是房间 ID 加根事件 ID。
    func draftKey(_ target: TimelineTarget) -> String {
        guard let root = target.threadRoot else { return target.roomId }
        return "\(target.roomId)|\(root)"
    }

    func acceptPendingDraft(in target: TimelineTarget) {
        guard let text = pendingDraft else { return }
        pendingDraft = nil
        appendDraft(text, to: target)
    }

    func appendDraft(_ text: String, to target: TimelineTarget) {
        let key = draftKey(target)
        let existing = drafts[key, default: ""]
        drafts[key] = existing.isEmpty ? text : existing + "\n" + text
        saveDraft(target)
    }

    /// 草稿在核心里，按时间线存（主对话和每个旁聊各一份），退出房间、重启 App 都还在。进时间线时读回来，
    /// 打完字写回去。
    func loadDraft(_ target: TimelineTarget) async {
        // `try?` 会把错误和“没有草稿”都折成 nil，这里要分开。
        let stored: ComposerDraft?
        do {
            stored = try await core.draft(target: target)
        } catch {
            return
        }
        let key = draftKey(target)
        // 读回来之前已经打了字、已经点了回复就不盖掉。
        if drafts[key, default: ""].isEmpty { drafts[key] = stored?.body ?? "" }
        if replies[key] == nil { replies[key] = stored?.reply }
    }

    /// 每次写草稿都带上正在回复的那条，不带核心会把存着的回复对象清掉。
    func saveDraft(_ target: TimelineTarget) {
        let key = draftKey(target)
        send(.timeline(action: TimelineAction.setDraft(
            target: target,
            body: drafts[key, default: ""], replyTo: replies[key]?.eventId
        )))
    }

    /// 回复按钮或右滑进入回复状态（ADR-0073）。
    func startReply(_ quote: ReplyQuote, in target: TimelineTarget) {
        replies[draftKey(target)] = quote
        saveDraft(target)
        replyFocusRevision += 1
    }

    func cancelReply(in target: TimelineTarget) {
        replies[draftKey(target)] = nil
        saveDraft(target)
    }

    /// 转发到另一个房间，随后打开它。
    func forward(_ itemId: String, from roomId: String, to target: String) {
        send(.timeline(action: TimelineAction.forwardMessage(
            target: TimelineTarget(roomId: roomId, threadRoot: nil),
            itemId: itemId, toRoomId: target
        )))
        openRoom(target)
    }

    /// 换了房间还在同一页：从应用页打开的就留在应用页，否则留在聊天页。
    func openRoom(_ roomId: String) {
        if let visiting {
            guard visiting.roomId != roomId else { return }
            self.visiting = VisitingRoom(tab: visiting.tab, roomId: roomId)
        } else if path.last != .room(roomId) {
            path = [.room(roomId)]
        }
    }

    /// 进 [roomId] 里的旁聊 [root]（ADR-0093）：主对话留在下面继续订阅，不重新订阅。
    func openSideChat(_ root: String, in roomId: String) {
        sideChats[roomId] = root
        composingSideChat = nil
    }

    func closeSideChat(in roomId: String) {
        sideChats[roomId] = nil
    }

    /// “新建聊天”选了 [contact]（ADR-0093 §4）：有能开旁聊的私聊就进去弹开新旁聊的面板；已经有和这个 AI
    /// 的私聊、但它不支持旁聊（能开旁聊的私聊在 `sideChatRoomId` 里），打开主对话并提示一句；其余照旧发起
    /// 或打开私聊。
    func startChat(with contact: ChatContact) {
        if let room = contact.sideChatRoomId,
           viewModel.rooms.contains(where: { $0.roomId == room && $0.membership == .joined }) {
            show(room, from: .chat)
            composingSideChat = room
            return
        }
        openDirectChat(with: contact.userId)
        if let opened = viewModel.rooms.first(where: { $0.directUserId == contact.userId && $0.membership == .joined }),
           viewModel.bots.contains(where: { $0.userId == opened.directUserId }) {
            noSideChatsRoom = opened.roomId
        }
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
        awaitedRoom = nil
        openingFailed = false
        visiting = nil
        path = []
        sideChats = [:]
        composingSideChat = nil
        noSideChatsRoom = nil
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

    /// 按房间 ID 打开（matrix.to 的房间和事件永久链接都落到房间）：已经在的就直接开；还没加入就先加入、等它出现。
    func openRoom(id: String) {
        awaitedPeer = nil
        awaitedRoom = nil
        visiting = nil
        if viewModel.rooms.contains(where: { $0.roomId == id && $0.membership == .joined }) {
            path = [.room(id)]
            return
        }
        do {
            try core.dispatch(action: .joinRoom(roomId: id))
            awaitedRoom = id
        } catch {
            rejection = error.problem
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
        if let room = awaitedRoom {
            if viewModel.rooms.contains(where: { $0.roomId == room && $0.membership == .joined }) {
                awaitedRoom = nil
                path = [.room(room)]
            } else if viewModel.problem != nil {
                awaitedRoom = nil
                openingFailed = true
            }
            return
        }
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
