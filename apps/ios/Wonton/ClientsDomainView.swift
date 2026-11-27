import SwiftUI
import UIKit
import WontonClient

/// “HTTPS 和域名”设置页（ADR-0089）：后缀、DNS 服务商、证书、CAA 和局域网记录都在客户端核心，
/// 这里只渲染 ViewModel、把表单值和操作发回核心。
struct ClientsDomainView: View {
    @State var model: ClientsDomainModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.wontonColors) var colors
    @Environment(\.openURL) var openURL
    @State private var suffix = ""
    @State var code = ""
    @State var otherCredentials = ""
    @State var credentials: [String: String] = [:]
    @State private var terms = false
    @State var copied: String?

    init(core: ClientCore) {
        _model = State(initialValue: ClientsDomainModel(core: core))
    }

    var viewModel: ClientsDomainViewModel { model.viewModel }

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen(String(localized: .Clients.clientsDomainTitle), usesSystemToolbar: true)
            .task { await model.observe() }
            .onDisappear { model.cancel() }
            .onChange(of: viewModel.suffix, initial: true) { _, value in suffix = value ?? "" }
            .onChange(of: viewModel.termsAccepted, initial: true) { _, value in terms = value }
    }

    @ViewBuilder private var content: some View {
        switch viewModel.state {
        case .loading:
            LoadingView()
        case let .failed(problem):
            ProblemView(problem: problem.problem) {
                model.send(.clients(action: .refreshDomain))
            }
        case .ready, .other:
            form
        }
    }

    private var problem: Problem? {
        if let rejection = model.rejection { return rejection }
        return viewModel.problem?.problem
    }

    private var form: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem {
                    ProblemNotice(problem: problem, onClose: { model.dismiss() })
                }
                suffixSection
                providerSection
                termsSection
                certificateSection
                caaSection
                lanSection
                if viewModel.desec { desecSection }
                saveButton
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .disabled(viewModel.saving)
    }

    private var suffixSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainSuffix)) {
            if viewModel.suffixEditable {
                TextField(String(localized: .Clients.clientsDomainSuffix), text: $suffix)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .wontonField()
                    .accessibilityIdentifier("clientsDomainSuffix")
                if !(viewModel.suffix ?? "").isEmpty {
                    secondary(String(localized: .Clients.clientsDomainSuffixNote))
                }
            } else if let suffix = viewModel.suffix {
                Text(verbatim: suffix)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .textSelection(.enabled)
            }
        }
    }

    private var termsSection: some View {
        WontonGroup {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Toggle(isOn: $terms) { Text(.Clients.clientsDomainTerms) }
                    .toggleStyle(WontonToggleStyle())
                    .accessibilityIdentifier("clientsDomainTerms")
                Button(String(localized: .Clients.clientsDomainTermsOpen)) {
                    openURL(URL(string: "https://letsencrypt.org/repository/")!)
                }
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textPrimary))
            }
            .padding(WontonTheme.spacing.s12)
        }
    }

    private var saveButton: some View {
        Button {
            save()
        } label: {
            if viewModel.saving {
                ProgressView()
            } else {
                Text(.Common.commonSave)
            }
        }
        .buttonStyle(WontonButtonStyle())
        .frame(maxWidth: .infinity)
        .disabled(viewModel.saving)
        .accessibilityIdentifier("clientsDomainSave")
    }

    private func save() {
        let inputs = viewModel.credentialVars.compactMap { name -> CredentialEntry? in
            let value = credentials[name] ?? ""
            return value.isEmpty ? nil : CredentialEntry(name: name, value: value)
        }
        let action = ClientsAction.saveDomain(
            suffix: viewModel.suffixEditable ? suffix : nil,
            provider: viewModel.otherProvider ? code : viewModel.selectedProvider,
            credentials: inputs,
            otherCredentials: viewModel.otherProvider ? otherCredentials : "",
            acceptTerms: terms
        )
        Task {
            if await model.save(.clients(action: action)) { dismiss() }
        }
    }
}

@MainActor
@Observable
final class ClientsDomainModel {
    private(set) var viewModel: ClientsDomainViewModel
    /// 核心当场拒绝的原因（校验没过）；失败由界面自己的提示条展示。
    var rejection: Problem?
    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.clientsDomain()
    }

    /// 进页面时取一次设置，之后跟着核心的推送走；页面消失时订阅结束。
    func observe() async {
        send(.clients(action: .refreshDomain))
        for await viewModel in core.subscribeClientsDomain().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
            rejection = nil
        } catch {
            rejection = error.problem
        }
    }

    /// 离开页面：丢掉没保存的表单。
    func cancel() {
        send(.clients(action: .cancelDomain))
    }

    /// 收起提示条：本机的当场拒绝和核心记的失败都清掉。
    func dismiss() {
        rejection = nil
        send(.clients(action: .dismissDomainProblem))
    }

    /// 保存：核心当场拒绝时返回 `false`；受理了就跟着状态走，落定后按有没有问题决定关不关页。
    func save(_ action: Action) async -> Bool {
        do {
            try core.dispatch(action: action)
            rejection = nil
        } catch {
            rejection = error.problem
            return false
        }
        for await viewModel in core.subscribeClientsDomain().updates() {
            self.viewModel = viewModel
            if !viewModel.saving { return viewModel.problem == nil }
        }
        return false
    }
}

extension ClientsProblem {
    var problem: Problem {
        switch kind {
        case .unauthenticated:
            categorized(
                title: .Clients.clientsDomainProblemUnauthenticated,
                nextStep: .Clients.clientsDomainProblemUnauthenticatedNext
            )
        case .invalidInput:
            Problem(title: String(localized: .Clients.clientsDomainProblemInvalid), detail: message)
        case .pluginUnavailable, .setupPending, .notRegistered, .coreNotInContainer:
            categorized(
                title: .Clients.clientsDomainProblemNotready,
                nextStep: .Clients.clientsDomainProblemNotreadyNext
            )
        case .credentialsFailed, .ingressUnavailable:
            categorized(
                title: .Clients.clientsDomainProblemUpstream,
                nextStep: .Clients.clientsDomainProblemUpstreamNext
            )
        case .conflict, .entryClosed, .publicAccessOff, .portQuotaExceeded:
            categorized(
                title: .Clients.clientsDomainProblemConflict,
                nextStep: .Clients.clientsDomainProblemConflictNext
            )
        case .lanUnavailable:
            categorized(
                title: .Clients.clientsDomainProblemLan,
                nextStep: .Clients.clientsDomainProblemLanNext
            )
        case .other:
            Problem(title: String(localized: .Clients.clientsDomainProblemFailed), detail: message)
        }
    }

    private func categorized(
        title: LocalizedStringResource,
        nextStep: LocalizedStringResource
    ) -> Problem {
        Problem(
            title: String(localized: title),
            nextStep: String(localized: nextStep),
            detail: message
        )
    }
}
