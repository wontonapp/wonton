import SwiftUI
import WontonClient

struct OnboardingModelStepView: View {
    let step: OnboardingModelStep
    let nav: StepNav
    let send: (Action) -> Void
    let openGateway: (URL) -> Void
    /// 键盘出现后把展开的服务商行（连同下面的密钥表单）滚到可视区顶部：视口顶边不随键盘移动，
    /// scrollTo 不会和安全区的收敛赛跑。
    let revealForm: (String) -> Void
    @State private var expandedProvider: String?
    @State private var showsMoreProviders = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            if let problem = step.problem {
                ProblemNotice(problem: problem.onboardingProblem)
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
                Button {
                    send(.continueOnboarding)
                } label: {
                    Text("用\(login.title)")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .disabled(!nav.canSkip)
            }
        }
    }

    @ViewBuilder private func providerList(_ list: ModelProviders) -> some View {
        if let providers = list.providers, !providers.isEmpty {
            ForEach(providers.filter(\.recommended), id: \.id) { providerRow($0) }
            if providers.contains(where: { !$0.recommended }) {
                Button {
                    showsMoreProviders.toggle()
                } label: {
                    HStack {
                        Text("更多服务商")
                        WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                            .rotationEffect(showsMoreProviders ? .degrees(90) : .zero)
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .accessibilityValue(showsMoreProviders ? "已展开" : "已收起")
                if showsMoreProviders {
                    ForEach(providers.filter { !$0.recommended }, id: \.id) { providerRow($0) }
                }
            }
        } else if let url = list.webUrl.flatMap(URL.init(string:)) {
            Button("打开模型设置") { openGateway(url) }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            if step.problem == nil {
                Button("重试") { send(.refreshOnboarding) }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
    }

    @ViewBuilder private func providerRow(_ provider: ModelProvider) -> some View {
        Button {
            expandedProvider = expandedProvider == provider.id ? nil : provider.id
        } label: {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonRow(title: provider.title)
                if provider.configured {
                    WontonIcon.check.view().foregroundStyle(Color(colors.success))
                }
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                    .rotationEffect(expandedProvider == provider.id ? .degrees(90) : .zero)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .padding(.trailing, WontonTheme.spacing.s16)
        }
        .buttonStyle(.plain)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
        .accessibilityValue(provider.configured ? "已配置" : "")
        // reveal 的滚动目标：钉住行，下面的表单跟着完整可见。
        .id(provider.id)
        if expandedProvider == provider.id {
            ProviderKeyForm(provider: provider, isSaving: step.saving == provider.id,
                            isBusy: !nav.canSkip, send: send, reveal: { revealForm(provider.id) })
        }
    }
}

private struct ProviderKeyForm: View {
    let provider: ModelProvider
    let isSaving: Bool
    let isBusy: Bool
    let send: (Action) -> Void
    let reveal: () -> Void
    @State private var key = ""
    @FocusState private var fieldFocused: Bool
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            SecureField("API key", text: input)
                .accessibilityLabel("API key")
                .accessibilityIdentifier("onboarding-provider-key")
                .textContentType(.none)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.plain)
                .wontonField()
                .disabled(isBusy)
                .submitLabel(.done)
                .focused($fieldFocused)
                // 键盘已经升起时切换服务商不再触发 keyboardDidShow，聚焦本身也要滚一次。
                .onChange(of: fieldFocused) { _, focused in
                    guard focused else { return }
                    reveal()
                }
                // 键盘刚弹出时可视区还没收窄，等它出现后再滚，输入框和按钮才一起在键盘上方。
                .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidShowNotification)) { _ in
                    guard fieldFocused else { return }
                    reveal()
                }
            HStack(spacing: WontonTheme.spacing.s12) {
                if let url = provider.keyUrl.flatMap(URL.init(string:)) {
                    Button("拿 key") { openURL(url) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
                Spacer(minLength: 0)
                Button(action: save) {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if isSaving { ProgressView() }
                        Text("保存并测试")
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .disabled(key.isEmpty || isBusy)
            }
        }
    }

    // 只有输入触发清除错误；发送后清空 key 不会顺带清掉刚返回的保存错误。
    private var input: Binding<String> {
        Binding(get: { key }, set: {
            key = $0
            send(.dismissModelProviderProblem)
        })
    }

    private func save() {
        send(.saveModelProviderKey(id: provider.id, key: key))
        key = ""
    }
}
