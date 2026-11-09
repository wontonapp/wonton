import Observation
import WontonClient

/// 外观和核心之间的桥：持有核心推来的 ViewModel，转发操作。
@MainActor
@Observable
final class AppearanceModel {
    private(set) var viewModel: AppearanceViewModel
    /// 核心拒绝保存的原因，设置页就地显示。
    private(set) var rejection: Problem?

    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.appearance()
    }

    /// 在根视图的 `.task` 里调用，App 在前台时一直订阅。
    func observe() async {
        for await viewModel in core.subscribeAppearance().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
            rejection = nil
        } catch {
            rejection = error.problem
        }
    }
}
