import SwiftUI
import WontonClient

struct OnboardingModelStepView: View {
    let step: OnboardingModelStep
    @Binding var form: OnboardingModelForm
    let send: (Action) -> Void
    let openGateway: (URL) -> Void
    let revealForm: (String) -> Void
    @State private var showsMoreProviders = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            if let problem = step.problem {
                ProblemNotice(problem: problem.onboardingProblem)
                    .id("onboarding-model-problem")
                if problem.reason != .keyRejected {
                    Button(String(localized: .Common.commonRetry)) { send(.refreshOnboarding) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            if let providers = step.providers {
                providerList(providers)
            } else if step.problem == nil {
                LoadingView().frame(maxWidth: .infinity)
            }
            ForEach(step.logins, id: \.id) { login in
                StepOptionRow(title: String(localized: .Onboarding5.onboardingModelUseLogin(login.title)),
                              isSelected: isLoginSelected(login.id)) {
                    form.expand(nil)
                    send(.chooseModelLogin(id: login.id))
                }
                .disabled(step.testing)
            }
        }
        .onAppear {
            if case let .provider(id, _, _) = step.choice,
               step.providers?.providers?.contains(where: { $0.id == id && !$0.recommended }) == true {
                showsMoreProviders = true
            }
        }
        .onChange(of: step.problem) { _, problem in
            if problem != nil { revealForm("onboarding-model-problem") }
        }
    }

    @ViewBuilder private func providerList(_ list: ModelProviders) -> some View {
        if !list.configured.isEmpty {
            StepOptionRow(title: String(localized: .Onboarding4.onboardingModelUseCurrent),
                          isSelected: step.choice == .current) {
                form.expand(nil)
                send(.useCurrentModel)
            }
            .disabled(step.testing)
            // 网关给的模型名带着内部地址，收进默认收起的“详情”。
            ProblemDetail(text: list.configured.map(\.title).formatted(.list(type: .and)))
                .padding(.leading, WontonTheme.spacing.s16)
        }
        if let providers = list.providers, !providers.isEmpty {
            ForEach(providers.filter(\.recommended), id: \.id) { providerRow($0) }
            if providers.contains(where: { !$0.recommended }) {
                Button {
                    showsMoreProviders.toggle()
                    if !showsMoreProviders, case let .preset(id) = form.expanded,
                       providers.contains(where: { $0.id == id && !$0.recommended }) {
                        form.expand(nil)
                    }
                } label: {
                    HStack {
                        Text(.Onboarding5.onboardingModelMore)
                        WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                            .rotationEffect(showsMoreProviders ? .degrees(90) : .zero)
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .accessibilityValue(showsMoreProviders
                    ? String(localized: .Onboarding1.onboardingModelExpanded)
                    : String(localized: .Onboarding2.onboardingModelCollapsed))
                .disabled(step.testing)
                if showsMoreProviders {
                    ForEach(providers.filter { !$0.recommended }, id: \.id) { providerRow($0) }
                }
            }
            if list.custom { customRow }
        } else if let url = list.webUrl.flatMap(URL.init(string:)) {
            Button(String(localized: .Onboarding5.onboardingModelOpenSettings)) { openGateway(url) }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
        }
    }

    @ViewBuilder private func providerRow(_ provider: ModelProvider) -> some View {
        let entry = OnboardingModelForm.Entry.preset(provider.id)
        StepOptionRow(title: provider.title, isSelected: isProviderSelected(provider.id), select: {
            form.expand(entry)
        }, trailing: { disclosure(isExpanded: form.expanded == entry) })
        .disabled(step.testing)
        if case let .provider(id, _, reachable) = step.choice, id == provider.id, !reachable {
            unreachable
        }
        if form.expanded == entry {
            OnboardingProviderForm(form: $form, keyURL: provider.keyUrl, testing: step.testing,
                                   send: send, reveal: revealForm)
                .id(entry)
        }
    }

    @ViewBuilder private var customRow: some View {
        StepOptionRow(title: String(localized: .Onboarding5.onboardingModelCustom), isSelected: isCustomSelected,
            select: {
            form.expand(.custom)
        }, trailing: { disclosure(isExpanded: form.expanded == .custom) })
        .disabled(step.testing)
        if case let .custom(_, _, reachable) = step.choice, !reachable { unreachable }
        if form.expanded == .custom {
            OnboardingProviderForm(form: $form, keyURL: nil, testing: step.testing,
                                   send: send, reveal: revealForm)
                .id(OnboardingModelForm.Entry.custom)
        }
    }

    private var unreachable: some View {
        Text(String(localized: .Onboarding4.onboardingModelUnreachable))
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.warning))
    }

    private func disclosure(isExpanded: Bool) -> some View {
        WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
            .rotationEffect(isExpanded ? .degrees(90) : .zero)
            .foregroundStyle(Color(colors.textSecondary))
            .accessibilityHidden(true)
    }

    private func isProviderSelected(_ id: String) -> Bool {
        if case let .provider(selected, _, _) = step.choice { return selected == id }
        return false
    }

    private var isCustomSelected: Bool {
        if case .custom = step.choice { return true }
        return false
    }

    private func isLoginSelected(_ id: String) -> Bool {
        if case let .login(selected, _) = step.choice { return selected == id }
        return false
    }
}
