import SwiftUI
import WontonClient

struct PrimaryAgentPicker: View {
    let model: OnboardingModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                if let problem = model.viewModel.problem {
                    ProblemView(problem: problem.problem) { model.send(.refreshOnboarding) }
                }
                if let problem = model.rejection {
                    ProblemNotice(problem: problem)
                }
                OnboardingAgentStep(agents: model.viewModel.agents) { model.send(.setPrimaryAgent(id: $0)) }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonModalScreen("默认 Agent")
        .task { model.send(.refreshOnboarding) }
    }
}
