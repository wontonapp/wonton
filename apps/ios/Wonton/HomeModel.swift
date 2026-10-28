import Observation
import WontonClient

/// “家”页面和核心之间的桥：持有核心推来的连接 ViewModel，把用户操作转发给核心。
/// 这里不做任何业务判断。
@MainActor
@Observable
final class HomeModel {
    private(set) var viewModel: HomeViewModel
    /// 核心拒绝上一个操作的原因，由界面弹窗展示。
    var rejection: String?

    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.home()
    }

    /// 在视图的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await viewModel in core.subscribeHome().updates() {
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
