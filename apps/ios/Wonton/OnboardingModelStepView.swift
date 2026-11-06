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
                    Button("重试") { send(.refreshOnboarding) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            if let providers = step.providers {
                providerList(providers)
            } else if step.problem == nil {
                LoadingView().frame(maxWidth: .infinity)
            }
            ForEach(step.logins, id: \.id) { login in
                StepOptionRow(title: "用\(login.title)", isSelected: isLoginSelected(login.id)) {
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
            StepOptionRow(title: "用现在的", description: list.configured.map(\.title).joined(separator: "、"),
                          isSelected: step.choice == .current) {
                form.expand(nil)
                send(.useCurrentModel)
            }
            .disabled(step.testing)
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
                        Text("更多服务商")
                        WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                            .rotationEffect(showsMoreProviders ? .degrees(90) : .zero)
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .accessibilityValue(showsMoreProviders ? "已展开" : "已收起")
                .disabled(step.testing)
                if showsMoreProviders {
                    ForEach(providers.filter { !$0.recommended }, id: \.id) { providerRow($0) }
                }
            }
            if list.custom { customRow }
        } else if let url = list.webUrl.flatMap(URL.init(string:)) {
            Button("打开模型设置") { openGateway(url) }
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
        StepOptionRow(title: "自定义", isSelected: isCustomSelected, select: {
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
        Text("连不上服务商，完成时照样保存。")
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
