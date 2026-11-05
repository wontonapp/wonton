import SwiftUI
import WontonClient

struct OnboardingView: View {
    let model: OnboardingModel
    let plugins: PluginsModel
    let nav: StepNav
    // 安装完成与取消由现有确认页区分；Agent 确认后立即继续，不等镜像下载。
    let confirmInstall: (PluginConfirmation, (() -> Void)?) -> Void
    let openGateway: (URL) -> Void
    @State private var profile = OwnerProfileFormState()
    @State private var isKeyboardVisible = false
    @State private var profileSave: Task<Void, Never>?
    @Environment(\.wontonColors) private var colors

    private var view: OnboardingViewModel { model.viewModel }

    var body: some View {
        StepCardFrame(
            nav: nav, title: view.title, primaryTitle: primaryTitle,
            isWorking: view.loading || profile.isSaving,
            back: { navigate(.backOnboardingStep) },
            skip: { navigate(.skipOnboardingStep) }, next: advance,
            content: {
                ScrollViewReader { proxy in
                    ScrollView {
                        content(proxy)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, WontonTheme.spacing.pageInset)
                            .padding(.bottom, WontonTheme.spacing.s16)
                    }
                    .scrollDismissesKeyboard(isKeyboardVisible ? .interactively : .never)
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
        .task {
            if view.step == .profile { await profile.load(core: model.core) }
        }
        .onChange(of: view.step) { _, step in
            profileSave?.cancel()
            if step == .profile {
                Task { await profile.load(core: model.core) }
            }
        }
        .onDisappear { profileSave?.cancel() }
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
                    OwnerProfileForm(state: profile) { Task { await profile.load(core: model.core) } }
                case .agent:
                    OnboardingAgentStep(agents: view.agents) { model.send(.chooseOnboardingAgent(id: $0)) }
                case .model:
                    OnboardingModelStepView(step: view.model, nav: nav, send: model.send, openGateway: openGateway,
                                            revealForm: { proxy.scrollTo($0, anchor: .top) })
                case .apps:
                    OnboardingAppsStep(needs: view.needs, agents: view.agents, catalogProblem: view.catalogProblem,
                                       plugins: plugins, send: model.send) { confirmation in
                        confirmInstall(confirmation, nil)
                    }
                case .finished, .other, nil:
                    EmptyView()
                }
            }
        }
    }

    private var primaryTitle: String {
        switch view.step {
        case .profile: "保存"
        case .apps: "完成"
        default: "下一步"
        }
    }

    private func advance() {
        switch view.step {
        case .profile:
            guard profileSave == nil else { return }
            profileSave = Task {
                defer { profileSave = nil }
                if await profile.save(core: model.core), !Task.isCancelled, model.viewModel.step == .profile {
                    model.send(.continueOnboarding)
                }
            }
        case .agent:
            if let source = view.agents.first(where: \.selected)?.source {
                confirmInstall(.install(source: source)) { model.send(.continueOnboarding) }
            } else {
                model.send(.continueOnboarding)
            }
        case .model, .apps:
            model.send(.continueOnboarding)
        case .finished, .other, nil:
            break
        }
    }

    private func navigate(_ action: Action) {
        profileSave?.cancel()
        model.send(action)
    }
}
