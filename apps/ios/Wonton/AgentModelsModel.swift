import Observation
import WontonClient

/// 模型页（ADR-0129）的状态都在核心：这里只转发 ViewModel 和点击。
@MainActor
@Observable
final class AgentModelsModel {
    private(set) var viewModel: AgentModelsViewModel
    /// 核心当场拒掉的动作（比如页面已经换了 Bot）；请求发出去以后的失败在 `viewModel.actionProblem`。
    var rejection: Problem?
    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.agentModels()
    }

    func observe() async {
        for await viewModel in core.subscribeAgentModels().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: AgentModelsAction) {
        do {
            try core.dispatch(action: .agentModels(action: action))
            rejection = nil
        } catch {
            rejection = error.problem
        }
    }
}
