import SwiftUI
import WontonClient

/// Shared by Bot and default-Bot tab entries before the existing WebKit container is allowed to load.
struct BotWebDisclosure: View {
    let url: String?
    let proceed: () -> Void
    let cancel: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                Text(.BotProfile0.botProfileWebDisclosureTitle)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("botWebDisclosureTitle")
                Text(.BotProfile1.botProfileWebDisclosureMessage)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textSecondary))
                    .accessibilityIdentifier("botWebDisclosureMessage")
                if let url {
                    Text(url)
                        .wontonText(WontonTheme.typography.supporting)
                        .textSelection(.enabled)
                        .accessibilityIdentifier("botWebDisclosureAddress")
                }
                Button(String(localized: .BotProfile0.botProfileWebOpen), action: proceed)
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                    .accessibilityIdentifier("botWebDisclosureContinue")
                Button(String(localized: .Common.commonCancel), action: cancel)
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                    .accessibilityIdentifier("botWebDisclosureCancel")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .accessibilityIdentifier("botWebDisclosure")
    }
}
