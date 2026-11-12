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
                        WontonRow(title: "系统通知", showsForward: true)
                    }
                    .buttonStyle(.plain)
                }
                Text("这个版本的 iOS App 还收不到推送")
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen("通知")
    }
}
