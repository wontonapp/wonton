import SwiftUI
import WontonClient

struct OnboardingView: View {
    let model: OnboardingModel
    let plugins: PluginsModel
    let nav: StepNav
    let openGateway: (ModelSettingsPage) -> Void
    /// 重看引导的关闭；首次引导没有。
    var close: (() -> Void)?
    @State private var isKeyboardVisible = false
    @Environment(\.wontonColors) private var colors

    private var view: OnboardingViewModel { model.viewModel }

    /// 关掉重看的引导：核心丢弃这次的选择，壳回到设置。X 和左边缘右滑走同一条路。
    private var handleClose: () -> Void {
        { model.close(); close?() }
    }

    var body: some View {
        StepCardFrame(
            nav: nav, title: view.step?.title ?? "", primaryTitle: primaryTitle,
            isWorking: view.loading || view.review.commit == .running,
            back: { model.send(.onboarding(action: .backOnboardingStep)) },
            skip: { model.send(.onboarding(action: .skipOnboardingStep)) }, next: advance,
            close: close.map { _ in handleClose },
            content: {
                ScrollViewReader { proxy in
                    ScrollView {
                        content(proxy)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, WontonTheme.spacing.pageInset)
                            .padding(.bottom, WontonTheme.spacing.s16)
                    }
                    .scrollDismissesKeyboard(isKeyboardVisible ? .interactively : .never)
                    .id(view.step)
                }
            }
        )
        .background(Color(colors.canvas))
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            isKeyboardVisible = true
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
            isKeyboardVisible = false
        }
        // 左边缘右滑：和系统返回同一路——能后退就后退，第一步（或关掉的引导）等同关闭。
        .background(
            FullscreenExitGesture(
                enabled: nav.canClose, webView: nil,
                exit: {
                    if nav.canBack {
                        model.send(.onboarding(action: .backOnboardingStep))
                    } else {
                        handleClose()
                    }
                }
            )
        )
    }

    @ViewBuilder private func content(_ proxy: ScrollViewProxy) -> some View {
        if let problem = view.problem {
            ProblemView(problem: problem.problem) { model.send(.onboarding(action: .refreshOnboarding)) }
        } else {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                if let problem = model.rejection {
                    ProblemNotice(problem: problem)
                }
                switch view.step {
                case .profile:
                    if view.loading {
                        LoadingView().frame(maxWidth: .infinity)
                    } else {
                        OnboardingProfileStepView(profile: view.profile, send: model.send)
                    }
                case .agent:
                    OnboardingAgentStep(agents: view.agents) {
                        model.send(.onboarding(action: .chooseOnboardingAgent(id: $0)))
                    }
                case .model:
                    OnboardingModelStepView(step: view.model, send: model.send,
                                            openGateway: openGateway,
                                            revealProblem: { proxy.scrollTo($0, anchor: .top) })
                case .apps:
                    OnboardingAppsStep(needs: view.needs, catalogProblem: view.catalogProblem,
                                       plugins: plugins, send: model.send)
                case .review:
                    OnboardingReviewStep(review: view.review, plugins: plugins, send: model.send)
                case .finished, .other, nil:
                    EmptyView()
                }
            }
        }
    }

    private var primaryTitle: String {
        guard view.step == .review else { return String(localized: .Onboarding2.onboardingNext) }
        switch view.review.commit {
        case .notStarted: return String(localized: .Onboarding3.onboardingDone)
        case .running: return String(localized: .Onboarding2.onboardingRunning)
        case .done: return String(localized: .Onboarding2.onboardingEnterChat)
        }
    }

    private func advance() {
        model.send(.onboarding(action: .continueOnboarding))
    }
}

/// 每一步卡片顶上的标题。
private extension OnboardingStep {
    var title: String {
        switch self {
        case .profile: String(localized: .Onboarding4.onboardingStepProfile)
        case .agent: String(localized: .Onboarding1.onboardingStepAgent)
        case .model: String(localized: .Onboarding1.onboardingStepModel)
        case .apps: String(localized: .Onboarding0.onboardingStepApps)
        case .review: String(localized: .Onboarding3.onboardingStepReview)
        case .finished: String(localized: .Onboarding0.onboardingStepFinished)
        case .other: String(localized: .Onboarding4.onboardingStepOther)
        }
    }
}
