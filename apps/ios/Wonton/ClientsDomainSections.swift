import SwiftUI

import WontonClient

extension ClientsDomainView {
    var providerSection: some View {
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
                    .foregroundStyle(Color(colors.textPrimary))
            }
        }
    }

    var signupLink: URL? {
        viewModel.providers
            .first { $0.code == viewModel.selectedProvider }
            .flatMap(\.signupUrl)
            .flatMap(URL.init(string:))
    }

    var certificateSection: some View {
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

    var caaSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainCaa)) {
            secondary(caaText, tint: viewModel.caa == .mismatch ? colors.warning : nil)
            if let record = viewModel.caaRecord {
                recordLine(record)
            }
        }
    }

    var caaText: String {
        switch viewModel.caa {
        case .bound: String(localized: .Clients.clientsDomainCaaBound)
        case .missing: String(localized: .Clients.clientsDomainCaaMissing)
        case .mismatch: String(localized: .Clients.clientsDomainCaaMismatch)
        case .unknown, .other: String(localized: .Clients.clientsDomainCaaUnknown)
        }
    }

    var lanSection: some View {
        DetailSection(title: String(localized: .Clients.clientsDomainLan)) {
            switch viewModel.lanRecord {
            case .ok: secondary(String(localized: .Clients.clientsDomainLanOk))
            case .missing: secondary(String(localized: .Clients.clientsDomainLanMissing), tint: colors.warning)
            case let .wrong(found):
                secondary(String(localized: .Clients.clientsDomainLanWrong(found)), tint: colors.warning)
            case .unknown, .other: secondary(String(localized: .Clients.clientsDomainLanUnknown))
            }
            if let name = viewModel.lanRecordName {
                let target = viewModel.lanTarget ?? String(localized: .Clients.clientsDomainLanAddress)
                recordLine("\(name) A \(target)")
            }
        }
    }

    func recordLine(_ record: String) -> some View {
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

    var desecSection: some View {
        DisclosureGroup {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                Text(.Clients.clientsDomainDesecSteps)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .fixedSize(horizontal: false, vertical: true)
                Button("desec.io") { openURL(URL(string: "https://desec.io")!) }
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textPrimary))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, WontonTheme.spacing.s8)
        } label: {
            Text(.Clients.clientsDomainDesec)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
        }
    }

    func secondary(_ text: String, tint: Rgb? = nil) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(tint ?? colors.textSecondary))
            .fixedSize(horizontal: false, vertical: true)
    }
}
