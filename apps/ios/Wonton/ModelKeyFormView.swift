import SwiftUI
import WontonClient

/// 填 key 接这一家（ADR-0129 第 3 节）：“去拿 key”在 App 内的 Safari 页打开，回来点系统的粘贴按钮；
/// App 不主动读剪贴板，主动读会弹询问。
struct ModelKeyFormView: View {
    let page: ModelProviderPage
    let form: ModelKeyForm
    /// 上一个动作没成的原因：`field` 是 `key`、`base_url` 时标在对应的输入框下面。
    let problem: AgentModelsProblem?
    let send: (AgentModelsAction) -> Void
    @State private var key = ""
    @State private var baseUrl: String
    @State private var keyPage: SafariTarget?
    @Environment(\.wontonColors) private var colors

    init(
        page: ModelProviderPage, form: ModelKeyForm, problem: AgentModelsProblem?,
        send: @escaping (AgentModelsAction) -> Void
    ) {
        self.page = page
        self.form = form
        self.problem = problem
        self.send = send
        _baseUrl = State(initialValue: form.baseUrl?.default ?? "")
    }

    private var trimmedKey: String { key.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var trimmedBaseUrl: String { baseUrl.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canSave: Bool {
        !page.busy && !trimmedKey.isEmpty && (form.baseUrl?.required != true || !trimmedBaseUrl.isEmpty)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if let field = form.baseUrl {
                TextField(
                    String(localized: field.required ? .AgentModels0.agentModelsKeyBaseUrl
                        : .AgentModels0.agentModelsKeyBaseUrlOptional),
                    text: $baseUrl
                )
                .keyboardType(.URL)
                .textContentType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.plain)
                .wontonField()
                fieldProblem("base_url")
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                // 不设 textContentType：设成密码会招来密码管理器的自动填充。
                SecureField(String(localized: .AgentModels0.agentModelsKeyField), text: $key)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .textFieldStyle(.plain)
                    .wontonField()
                PasteButton(payloadType: String.self) { strings in
                    guard let pasted = strings.first else { return }
                    Task { @MainActor in key = pasted.trimmingCharacters(in: .whitespacesAndNewlines) }
                }
                .labelStyle(.iconOnly)
                .buttonBorderShape(.capsule)
                .tint(Color(colors.primary))
            }
            if page.keyCheck?.check == .rejected {
                ProblemNotice(problem: Problem(
                    title: String(localized: .AgentModels1.agentModelsKeyRejected),
                    nextStep: String(localized: .AgentModels1.agentModelsKeyRejectedNext),
                    detail: page.keyCheck?.message
                ))
            }
            fieldProblem("key")
            Button(action: save) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    if page.busy { ProgressView().tint(Color(colors.onPrimary)) }
                    Text(.Common.commonSave)
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(!canSave)
            if let link = form.keyUrl.flatMap(URL.init(string:)) {
                Button { keyPage = SafariTarget(url: link) } label: {
                    Text(.AgentModels0.agentModelsKeyGet).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
        .sheet(item: $keyPage) { target in
            SafariView(url: target.url).ignoresSafeArea()
        }
    }

    @ViewBuilder private func fieldProblem(_ field: String) -> some View {
        if let problem, problem.field == field {
            let shown = problem.problem
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(verbatim: shown.title)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
                if let detail = shown.detail { ProblemDetail(text: detail) }
            }
        }
    }

    private func save() {
        let address = trimmedBaseUrl
        send(.saveModelKey(key: trimmedKey, baseUrl: form.baseUrl == nil || address.isEmpty ? nil : address))
    }
}
