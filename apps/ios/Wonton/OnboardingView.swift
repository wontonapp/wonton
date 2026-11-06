import SwiftUI
import WontonClient

struct OnboardingView: View {
    let model: OnboardingModel
    let plugins: PluginsModel
    let nav: StepNav
    let openGateway: (URL) -> Void
    @State private var modelForm = OnboardingModelForm()
    @State private var isKeyboardVisible = false
    @Environment(\.wontonColors) private var colors

    private var view: OnboardingViewModel { model.viewModel }

    private var frameNav: StepNav {
        var result = nav
        // key 在测试前不进核心，只有这个按钮需要合并输入框状态。
        if view.step == .model, modelForm.hasCompleteInput, !view.model.testing {
            result.canNext = true
        }
        return result
    }

    var body: some View {
        StepCardFrame(
            nav: frameNav, title: view.step?.title ?? "", primaryTitle: primaryTitle,
            isWorking: view.loading || view.model.testing || view.review.commit == .running,
            back: { model.send(.backOnboardingStep) },
            skip: { model.send(.skipOnboardingStep) }, next: advance,
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
        .onChange(of: view.step, initial: true) { _, step in
            modelForm = OnboardingModelForm(choice: step == .model ? view.model.choice : nil)
        }
    }

    @ViewBuilder private func content(_ proxy: ScrollViewProxy) -> some View {
        if let problem = view.problem {
            ProblemView(problem: problem.problem) { model.send(.refreshOnboarding) }
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
                    OnboardingAgentStep(agents: view.agents) { model.send(.chooseOnboardingAgent(id: $0)) }
                case .model:
                    OnboardingModelStepView(step: view.model, form: $modelForm, send: model.send,
                                            openGateway: openGateway,
                                            revealForm: { proxy.scrollTo($0, anchor: .top) })
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
        guard view.step == .review else { return String(localized: .onboardingNext) }
        switch view.review.commit {
        case .notStarted: return String(localized: .onboardingDone)
        case .running: return String(localized: .onboardingRunning)
        case .done: return String(localized: .onboardingEnterChat)
        }
    }

    private func advance() {
        if view.step == .model {
            modelForm.submit(thenContinue: true, send: model.send)
        } else {
            model.send(.continueOnboarding)
        }
    }
}

/// 每一步卡片顶上的标题。
private extension OnboardingStep {
    var title: String {
        switch self {
        case .profile: String(localized: .onboardingStepProfile)
        case .agent: String(localized: .onboardingStepAgent)
        case .model: String(localized: .onboardingStepModel)
        case .apps: String(localized: .onboardingStepApps)
        case .review: String(localized: .onboardingStepReview)
        case .finished: String(localized: .onboardingStepFinished)
        case .other: String(localized: .onboardingStepOther)
        }
    }
}
