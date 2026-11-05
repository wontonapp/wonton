import SwiftUI
import WontonClient

struct OnboardingGate<Content: View>: View {
    let model: OnboardingModel
    let plugins: PluginsModel
    let isPaired: Bool
    let finished: (String?) -> Void
    let openGateway: (URL) -> Void
    @ViewBuilder let content: Content
    @State private var installation: OnboardingInstallation?

    var body: some View {
        Group {
            if !isPaired {
                content
            } else {
                pairedContent
            }
        }
        .environment(model)
        .task { await model.observe() }
        .fullScreenCover(item: $installation) { request in
            if let confirmed = request.confirmed {
                InstallConfirmView(model: plugins, confirmation: request.confirmation, confirmed: {
                    installation = nil
                    confirmed()
                })
            } else {
                InstallConfirmView(model: plugins, confirmation: request.confirmation)
            }
        }
        .onChange(of: model.viewModel.step) { previous, step in
            guard isPaired, step == .finished else { return }
            switch previous {
            case .profile, .agent, .model, .apps:
                finished(model.viewModel.primaryBotUserId)
            case .finished, .other, nil:
                break
            }
        }
    }

    @ViewBuilder private var pairedContent: some View {
        switch model.viewModel.step {
        case .profile, .agent, .model, .apps:
            // 活跃步骤必须有 nav；缺失是核心契约错误，不能在壳里猜步骤或按钮状态。
            OnboardingView(model: model, plugins: plugins, nav: model.viewModel.nav!, confirmInstall: {
                installation = OnboardingInstallation(confirmation: $0, confirmed: $1)
            }, openGateway: openGateway)
        case nil where model.viewModel.loading:
            LoadingView().frame(maxWidth: .infinity, maxHeight: .infinity)
        case nil, .other, .finished:
            content
                .modifier(ReplacedAgentOffer(model: model, plugins: plugins))
        }
    }
}

private struct OnboardingInstallation: Identifiable {
    let confirmation: PluginConfirmation
    let confirmed: (() -> Void)?
    var id: String { confirmation.id }
}
