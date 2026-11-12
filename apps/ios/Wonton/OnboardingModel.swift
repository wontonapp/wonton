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
            if case .restartOnboarding = action { skipsUnavailableGate = false }
        } catch {
            rejection = error.problem
        }
    }
}
