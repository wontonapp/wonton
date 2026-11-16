import SwiftUI
import UIKit

struct NotificationSettingsView: View {
    @Environment(\.openURL) private var openURL
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    Button {
                        if let url = URL(string: UIApplication.openNotificationSettingsURLString) {
                            openURL(url)
                        }
                    } label: {
                        WontonRow(title: String(localized: .Settings.notificationSystemSettings), showsForward: true)
                    }
                    .buttonStyle(.plain)
                }
                Text(.Settings.notificationUnsupported)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .Settings.notificationTitle), usesSystemToolbar: true)
    }
}
