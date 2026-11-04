import Observation
import WontonClient

@MainActor
@Observable
final class ChatModel {
    private(set) var viewModel: ChatViewModel
    var rejection: String?
    /// 聊天页打开的房间。主屏上点 Agent 插件时直接打开和它的 Bot 的私聊。
    var path: [String] = []
    /// 刚发起了和这个用户的私聊，房间出现后打开。
    @ObservationIgnored private var awaitedPeer: String?

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

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = String(describing: error)
        }
    }

    /// 打开和 `userId` 的私聊：已经加入的直接打开；还没有就发起，房间出现后打开；对方的邀请还没接受时停在列表上，
    /// 邀请就在那里。
    func openDirectChat(with userId: String) {
        awaitedPeer = nil
        path = []
        let rooms = viewModel.rooms.filter { $0.directUserId == userId }
        if let joined = rooms.first(where: { $0.membership == .joined }) {
            path = [joined.roomId]
        } else if rooms.isEmpty {
            do {
                try core.dispatch(action: .startDirectChat(userId: userId))
                awaitedPeer = userId
            } catch {
                rejection = String(describing: error)
            }
        }
    }

    private func openAwaitedChat() {
        guard let peer = awaitedPeer,
              let room = viewModel.rooms.first(where: { $0.directUserId == peer && $0.membership == .joined })
        else { return }
        awaitedPeer = nil
        path = [room.roomId]
    }
}
