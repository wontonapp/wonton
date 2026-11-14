import Observation
import WontonClient

@MainActor
@Observable
final class OnboardingModel {
    private(set) var viewModel: OnboardingViewModel
    var rejection: Problem?
    var skipsUnavailableGate = false
    let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.onboarding()
    }

    func observe() async {
        for await viewModel in core.subscribeOnboarding().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
            rejection = nil
            if case .onboarding(action: .restartOnboarding) = action { skipsUnavailableGate = false }
        } catch {
            rejection = error.problem
        }
    }

    /// 关掉重看的引导：不保存任何东西，壳负责回到设置；经统一 `Action.onboarding` 入口发送。
    func close() {
        send(.onboarding(action: .closeOnboarding))
    }
}
