import SwiftUI
import WontonClient

struct OnboardingAgentStep: View {
    let agents: [OnboardingAgentRow]
    let choose: (String) -> Void

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            ForEach(agents, id: \.id) { agent in
                StepOptionRow(
                    title: agent.title, description: connection(agent.logins),
                    isSelected: agent.selected, blocked: agent.blocked
                ) {
                    choose(agent.id)
                }
            }
        }
    }

    /// 接模型的方式，如“ChatGPT 账号或 key”；没有账号登录时只有 key。
    private func connection(_ logins: [String]) -> String {
        guard !logins.isEmpty else { return String(localized: .Onboarding0.onboardingAgentConnectionKey) }
        return String(localized: .Onboarding4.onboardingAgentConnectionLogins(logins.formatted(.list(type: .or))))
    }
}
