import SwiftUI
import WontonClient

struct OnboardingGate<Content: View>: View {
    let model: OnboardingModel
    let plugins: PluginsModel
    let isPaired: Bool
    let finished: (String?) -> Void
    let openGateway: (URL) -> Void
    @ViewBuilder let content: Content

    var body: some View {
        Group {
            if !isPaired || model.skipsUnavailableGate {
                content
            } else {
                pairedContent
            }
        }
        .environment(model)
        .task { await model.observe() }
        .onChange(of: model.viewModel.step) { previous, step in
            guard isPaired, !model.skipsUnavailableGate, step == .finished else { return }
            switch previous {
            case .profile, .agent, .model, .apps, .review:
                finished(model.viewModel.primaryBotUserId)
            case .finished, .other, nil:
                break
            }
        }
    }

    @ViewBuilder private var pairedContent: some View {
        switch model.viewModel.step {
        case .profile, .agent, .model, .apps, .review:
            // 活跃步骤必须有 nav；缺失是核心契约错误，不能在壳里猜步骤或按钮状态。
            OnboardingView(model: model, plugins: plugins, nav: model.viewModel.nav!, openGateway: openGateway)
        case nil where model.viewModel.loading:
            LoadingView().frame(maxWidth: .infinity, maxHeight: .infinity)
        case nil where model.viewModel.problem != nil:
            VStack(spacing: WontonTheme.spacing.s8) {
                Spacer(minLength: 0)
                ProblemView(problem: model.rejection ?? model.viewModel.problem!.problem) {
                    model.send(.refreshOnboarding)
                }
                Button(String(localized: .Onboarding2.onboardingSkipAnyway)) { model.skipsUnavailableGate = true }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case nil, .other, .finished:
            content
                .modifier(ReplacedAgentOffer(model: model, plugins: plugins))
        }
    }
}
