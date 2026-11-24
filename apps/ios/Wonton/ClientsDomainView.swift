import SwiftUI
import UIKit
import WontonClient

/// “HTTPS 和域名”设置页（ADR-0089）：后缀、DNS 服务商、证书、CAA 和局域网记录都在客户端核心，
/// 这里只渲染 ViewModel、把表单值和操作发回核心。
struct ClientsDomainView: View {
    @State private var model: ClientsDomainModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.wontonColors) private var colors
    @Environment(\.openURL) private var openURL
    @State private var suffix = ""
    @State private var code = ""
    @State private var otherCredentials = ""
    @State private var credentials: [String: String] = [:]
    @State private var terms = false
    @State private var copied: String?

    init(core: ClientCore) {
        _model = State(initialValue: ClientsDomainModel(core: core))
    }

    private var viewModel: ClientsDomainViewModel { model.viewModel }

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen(String(localized: .Clients.clientsDomainTitle), usesSystemToolbar: true)
            .task { await model.observe() }
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

    private var providerSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainProvider)) {
            ForEach(Array(viewModel.providers.enumerated()), id: \.offset) { _, provider in
                if let providerCode = provider.code {
                    StepOptionRow(
                        title: provider.name,
                        isSelected: viewModel.selectedProvider == providerCode
                    ) {
                        model.send(.clients(action: .selectDomainProvider(code: providerCode)))
                    }
                } else {
                    StepOptionRow(
                        title: String(localized: .Clients.clientsDomainOther),
                        isSelected: viewModel.otherProvider
                    ) {
                        model.send(.clients(action: .selectDomainProvider(code: nil)))
                    }
                    .accessibilityIdentifier("clientsDomainOther")
                }
            }
            if viewModel.otherProvider {
                TextField(String(localized: .Clients.clientsDomainOtherCode), text: $code)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                    .accessibilityIdentifier("clientsDomainOtherCode")
                TextField(
                    String(localized: .Clients.clientsDomainCredentials),
                    text: $otherCredentials,
                    prompt: Text(String(localized: .Clients.clientsDomainOtherHint)),
                    axis: .vertical
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(3...)
                .wontonField()
                .accessibilityIdentifier("clientsDomainOtherCredentials")
            } else if !viewModel.credentialVars.isEmpty {
                SecretInputsSection(
                    inputs: viewModel.credentialVars.map {
                        SecretInput(name: $0, title: $0, description: nil, url: nil, required: true)
                    },
                    values: credentials
                ) { name, value in
                    credentials[name] = value
                }
                if viewModel.credentialsSet {
                    secondary(String(localized: .Clients.clientsDomainCredentialsSet))
                }
            }
            if let link = signupLink {
                Button(String(localized: .Clients.clientsDomainSignup)) { openURL(link) }
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.accentInk))
            }
        }
    }

    private var signupLink: URL? {
        viewModel.providers
            .first { $0.code == viewModel.selectedProvider }
            .flatMap(\.signupUrl)
            .flatMap(URL.init(string:))
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
                .foregroundStyle(Color(colors.accentInk))
            }
            .padding(WontonTheme.spacing.s12)
        }
    }

    private var certificateSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainCertificate)) {
            switch viewModel.certificate.state {
            case let .valid(notAfterMs):
                secondary(
                    String(
                        localized: .Clients.clientsDomainCertValid(
                            Date(timeIntervalSince1970: Double(notAfterMs) / 1000)
                                .formatted(date: .abbreviated, time: .shortened)
                        )
                    )
                )
            case .renewing:
                secondary(String(localized: .Clients.clientsDomainCertRenewing), tint: colors.warning)
            case .expired:
                secondary(String(localized: .Clients.clientsDomainCertExpired), tint: colors.danger)
            case .issuing:
                secondary(String(localized: .Clients.clientsDomainCertIssuing))
            case .none, .other:
                secondary(String(localized: .Clients.clientsDomainCertNone))
            }
            if viewModel.certificate.retryable {
                Button(String(localized: .Clients.clientsDomainRetry)) {
                    model.send(.clients(action: .retryCertificate))
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            if let details = viewModel.certificate.details {
                ProblemDetail(text: details)
            }
        }
    }

    private var caaSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainCaa)) {
            secondary(caaText, tint: viewModel.caa == .mismatch ? colors.warning : nil)
            if let record = viewModel.caaRecord {
                recordLine(record)
            }
        }
    }

    private var caaText: String {
        switch viewModel.caa {
        case .bound: String(localized: .Clients.clientsDomainCaaBound)
        case .missing: String(localized: .Clients.clientsDomainCaaMissing)
        case .mismatch: String(localized: .Clients.clientsDomainCaaMismatch)
        case .unknown, .other: String(localized: .Clients.clientsDomainCaaUnknown)
        }
    }

    private var lanSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainLan)) {
            switch viewModel.lanRecord {
            case .ok: secondary(String(localized: .Clients.clientsDomainLanOk))
            case .missing: secondary(String(localized: .Clients.clientsDomainLanMissing), tint: colors.warning)
            case let .wrong(found):
                secondary(String(localized: .Clients.clientsDomainLanWrong(found)), tint: colors.warning)
            case .unknown, .other: secondary(String(localized: .Clients.clientsDomainLanUnknown))
            }
            if let name = viewModel.lanRecordName {
                recordLine("\(name) A \(String(localized: .Clients.clientsDomainLanAddress))")
            }
        }
    }

    private func recordLine(_ record: String) -> some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            Text(verbatim: record)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .textSelection(.enabled)
                .accessibilityIdentifier("clientsDomainRecord")
            Button {
                UIPasteboard.general.string = record
                copied = record
                Task {
                    try? await Task.sleep(for: .seconds(2))
                    if copied == record { copied = nil }
                }
            } label: {
                WontonIcon.document.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .accessibilityLabel(Text(.Clients.clientsDomainCopy))
            Spacer(minLength: 0)
        }
        .overlay(alignment: .trailing) {
            if copied == record {
                Text(.Clients.clientsDomainCopied)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
    }

    private var desecSection: some View {
        DisclosureGroup {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Text(.Clients.clientsDomainDesecSteps)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .fixedSize(horizontal: false, vertical: true)
                Button("desec.io") { openURL(URL(string: "https://desec.io")!) }
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.accentInk))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, WontonTheme.spacing.s8)
        } label: {
            Text(.Clients.clientsDomainDesec)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
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

    private func secondary(_ text: String, tint: Rgb? = nil) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(tint ?? colors.textSecondary))
            .fixedSize(horizontal: false, vertical: true)
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
