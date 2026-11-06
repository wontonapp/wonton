import SwiftUI
import WontonClient

struct OnboardingTipView: View {
    let text: String
    let dismiss: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            Text(verbatim: text)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity, alignment: .leading)
            WontonIconButton(icon: .close, label: String(localized: .commonClose), action: dismiss)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }
}

struct OnboardingChatPrompts: View {
    let prompts: [String]
    let draft: (String) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s8) {
                ForEach(Array(prompts.enumerated()), id: \.offset) { _, prompt in
                    Button { draft(prompt) } label: { Text(verbatim: prompt) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }
}

struct ReplacedAgentOffer: ViewModifier {
    let model: OnboardingModel
    let plugins: PluginsModel
    @State private var offered: ReplacedAgent?
    @State private var uninstalling: UninstallTarget?
    @State private var answeredId: String?

    func body(content: Content) -> some View {
        let agent = offered
        content
            .wontonDialog(
                String(localized: .onboardingReplacedAgentTitle(offered?.title ?? "")),
                isPresented: Binding(get: { offered != nil }, set: { if !$0 { offered = nil } }),
                actions: [
                    WontonDialogAction(title: String(localized: .onboardingUninstall)) {
                        answer(agent, uninstall: true)
                    },
                    WontonDialogAction(title: String(localized: .onboardingKeep)) {
                        answer(agent, uninstall: false)
                    }
                ],
                onDismiss: { answer(agent, uninstall: false) }
            )
            .uninstallDialog($uninstalling, model: plugins)
            .onChange(of: model.viewModel.replacedAgent, initial: true) { _, agent in
                if let agent, agent.id != answeredId { offered = agent }
            }
    }

    private func answer(_ agent: ReplacedAgent?, uninstall: Bool) {
        guard let agent else { return }
        answeredId = agent.id
        model.send(.dismissReplacedAgentOffer)
        if uninstall { uninstalling = UninstallTarget(id: agent.id, title: agent.title) }
    }
}
