import Observation
import WontonClient

/// “等你确认”列表的模型：订阅核心的列表，壳只读；问题不弹窗，列表页照实显示并给重试。
@MainActor
@Observable
final class ConfirmationsModel {
    private(set) var viewModel: ConfirmationsViewModel

    let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.confirmations()
    }

    #if DEBUG
        /// 截图测试拿固定 ViewModel 画界面，不订阅核心推送。
        init(core: ClientCore, viewModel: ConfirmationsViewModel) {
            self.core = core
            self.viewModel = viewModel
        }
    #endif

    /// 在视图的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await _ in core.subscribeConfirmations().updates() {
            viewModel = core.confirmations()
        }
    }
}
