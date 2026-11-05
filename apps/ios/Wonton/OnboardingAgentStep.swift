import SwiftUI
import WontonClient

struct OnboardingAgentStep: View {
    let agents: [OnboardingAgentRow]
    let choose: (String) -> Void

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            ForEach(agents, id: \.id) { agent in
                StepOptionRow(
                    title: agent.title, description: agent.connection,
                    isSelected: agent.selected, blocked: agent.blocked
                ) {
                    choose(agent.id)
                }
            }
        }
    }
}
