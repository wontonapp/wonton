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
        for await viewModel in core.subscribeChat().updates() {
            self.viewModel = viewModel
            openAwaitedChat()
        }
    }

    /// 发起了私聊，房间还没出现。
    var isOpeningChat: Bool { awaitedPeer != nil }

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

    /// 从 `origin` 这一页打开和 `userId` 的私聊：已经加入的直接打开；还没有就发起，房间出现后打开，这期间留在原页；
    /// 对方的邀请还没接受时停在列表上，邀请就在那里。返回主界面要不要切到聊天页。
    @discardableResult
    func openDirectChat(with userId: String, from origin: AppTab = .chat) -> Bool {
        awaitedPeer = nil
        path = []
        visiting = nil
        let rooms = viewModel.rooms.filter { $0.directUserId == userId }
        if let joined = rooms.first(where: { $0.membership == .joined }) {
            show(joined.roomId, from: origin)
            return origin == .chat
        }
        guard rooms.isEmpty else { return true }
        do {
            try core.dispatch(action: .startDirectChat(userId: userId))
            awaitedPeer = userId
            awaitedFrom = origin
            return origin == .chat
        } catch {
            rejection = error.problem
            return true
        }
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
