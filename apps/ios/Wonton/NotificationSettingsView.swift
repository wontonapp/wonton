import SwiftUI
import UIKit

/// 设置里的通知页：系统通知的入口和系统里的状态。这一版还收不到推送，能用的是系统那一层的开关，
/// 所以入口先把权限问到手，再去系统设置里云吞自己的那一页。
struct NotificationSettingsView: View {
    @State private var permission = NotificationPermission.notAsked
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    Button {
                        Task {
                            if permission.asksBeforeOpening { await NotificationPermission.ask() }
                            permission = await NotificationPermission.current()
                            await NotificationPermission.openSystemSettings()
                        }
                    } label: {
                        WontonRow(
                            title: String(localized: .Settings.notificationSystemSettings),
                            value: permission.rowValue,
                            showsForward: true
                        )
                    }
                    .buttonStyle(.plain)
                }
                Text(.Settings.notificationUnsupported)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .task { permission = await NotificationPermission.current() }
        // 从系统设置回来时那边可能刚改过开关，回到前台重读一次状态。
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { permission = await NotificationPermission.current() } }
        }
        .wontonPushedScreen(String(localized: .Settings.notificationTitle), usesSystemToolbar: true)
    }
}
