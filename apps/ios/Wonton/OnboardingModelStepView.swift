import SwiftUI
import WontonClient

struct OnboardingModelStepView: View {
    let step: OnboardingModelStep
    let send: (Action) -> Void
    let openGateway: (ModelSettingsPage) -> Void
    let revealProblem: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            if let problem = step.problem {
                ProblemNotice(problem: problem.onboardingProblem)
                    .id("onboarding-model-problem")
                Button(String(localized: .Common.commonRetry)) { send(.refreshOnboarding) }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            if let settings = step.settings {
                Button(String(localized: .Onboarding5.onboardingModelOpenSettings)) { openGateway(settings) }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            } else if step.settings == nil, step.problem == nil {
                LoadingView().frame(maxWidth: .infinity)
            }
            if step.hasConfiguredModels {
                StepOptionRow(title: String(localized: .Onboarding4.onboardingModelUseCurrent),
                              isSelected: step.choice == .current) {
                    send(.useCurrentModel)
                }
            }
            ForEach(step.logins, id: \.id) { login in
                StepOptionRow(title: String(localized: .Onboarding5.onboardingModelUseLogin(login.title)),
                              isSelected: isLoginSelected(login.id)) {
                    send(.chooseModelLogin(id: login.id))
                }
            }
        }
        .onChange(of: step.problem) { _, problem in
            if problem != nil { revealProblem("onboarding-model-problem") }
        }
    }

    private func isLoginSelected(_ id: String) -> Bool {
        if case let .login(selected, _) = step.choice { return selected == id }
        return false
    }
}
