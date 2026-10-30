import Observation
import WontonClient

@MainActor
@Observable
final class ChatModel {
    private(set) var viewModel: ChatViewModel
    var rejection: String?

    let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.chat()
    }

    /// 在视图的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await viewModel in core.subscribeChat().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = String(describing: error)
        }
    }
}
