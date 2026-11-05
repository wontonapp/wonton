import SwiftUI
import WontonClient

struct OnboardingSettings: View {
    let model: OnboardingModel
    @State private var isChoosingAgent = false

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            if let problem = model.rejection ?? model.viewModel.problem?.problem {
                ProblemRow(problem: problem) { model.send(.refreshOnboarding) }
            }
            WontonGroup {
                Button { model.send(.restartOnboarding) } label: {
                    WontonRow(title: "重看引导")
                }
                .buttonStyle(.plain)
                WontonSeparator()
                Button { isChoosingAgent = true } label: {
                    WontonRow(title: "默认 Agent", showsForward: true)
                }
                .buttonStyle(.plain)
            }
        }
        .fullScreenCover(isPresented: $isChoosingAgent) {
            PrimaryAgentPicker(model: model)
        }
    }
}

private struct PrimaryAgentPicker: View {
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
