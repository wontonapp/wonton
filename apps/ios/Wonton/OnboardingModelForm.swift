import SwiftUI
import WontonClient

struct OnboardingModelForm {
    enum Entry: Hashable {
        case preset(String)
        case custom
    }

    var expanded: Entry?
    var baseURL = ""
    var model = ""
    var key = ""

    init(choice: OnboardingModelChoice? = nil) {
        switch choice {
        case let .provider(id, _, _):
            expanded = .preset(id)
        case let .custom(baseURL, model, _):
            expanded = .custom
            self.baseURL = baseURL
            self.model = model
        default:
            break
        }
    }

    var hasCompleteInput: Bool {
        guard !key.isEmpty else { return false }
        switch expanded {
        case .preset: return true
        case .custom: return !baseURL.isEmpty && !model.isEmpty
        case nil: return false
        }
    }

    mutating func expand(_ entry: Entry?) {
        key = ""
        expanded = expanded == entry ? nil : entry
    }

    mutating func submit(thenContinue: Bool, send: (Action) -> Void) {
        guard !key.isEmpty, let expanded else {
            if thenContinue { send(.continueOnboarding) }
            return
        }
        let input: ModelProviderInput = switch expanded {
        case let .preset(id): .preset(id: id, key: key)
        case .custom: .custom(baseUrl: baseURL, model: model, key: key)
        }
        key = ""
        send(.testModelProvider(input: input, thenContinue: thenContinue))
    }
}

struct OnboardingProviderForm: View {
    @Binding var form: OnboardingModelForm
    let keyURL: String?
    let testing: Bool
    let send: (Action) -> Void
    let reveal: (String) -> Void
    @FocusState private var focusedField: String?
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if form.expanded == .custom {
                TextField(String(localized: .Onboarding0.onboardingModelFormBaseUrl), text: input(\.baseURL),
                          prompt: Text("https://"))
                    .keyboardType(.URL)
                    .focused($focusedField, equals: "onboarding-model-url")
                    .id("onboarding-model-url")
                    .wontonField()
                TextField(String(localized: .Onboarding0.onboardingModelFormModelName), text: input(\.model))
                    .focused($focusedField, equals: "onboarding-model-name")
                    .id("onboarding-model-name")
                    .wontonField()
            }
            SecureField(String(localized: .Onboarding3.onboardingModelFormApiKey), text: input(\.key))
                .accessibilityLabel(String(localized: .Onboarding3.onboardingModelFormApiKey))
                .accessibilityIdentifier("onboarding-provider-key")
                .textContentType(.none)
                .submitLabel(.done)
                .focused($focusedField, equals: "onboarding-provider-key")
                .id("onboarding-provider-key")
                .wontonField()
            HStack(spacing: WontonTheme.spacing.s12) {
                if let url = keyURL.flatMap(URL.init(string:)) {
                    Button(String(localized: .Onboarding4.onboardingModelGetKey)) { openURL(url) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
                Spacer(minLength: 0)
                Button {
                    form.submit(thenContinue: false, send: send)
                } label: {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if testing { ProgressView() }
                        Text(.Onboarding5.onboardingModelTest)
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .disabled(form.key.isEmpty || testing)
            }
        }
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .textFieldStyle(.plain)
        .disabled(testing)
        .onChange(of: focusedField) { _, field in
            if let field { reveal(field) }
        }
        // 键盘收窄视口后再滚一次；自定义表单逐个定位输入框，不把整张表单压在键盘下面。
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidShowNotification)) { _ in
            if let focusedField { reveal(focusedField) }
        }
    }

    private func input(_ path: WritableKeyPath<OnboardingModelForm, String>) -> Binding<String> {
        Binding(get: { form[keyPath: path] }, set: {
            form[keyPath: path] = $0
            send(.dismissModelProviderProblem)
        })
    }
}
