import SwiftUI
import UIKit
import WontonClient

struct NotificationSettingsView: View {
    @Environment(\.openURL) private var openURL
    let settings: SettingsModel
    let push: PushModel

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
                    WontonSeparator()
                    // 收新消息的方式 iOS 只有系统推送（ADR-0003），没有 Android 那个保持连接的开关。
                    WontonRow(title: String(localized: .Settings.notificationPush), value: line)
                }
                if let detail {
                    ProblemDetail(text: detail)
                }

            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .Settings.notificationTitle), usesSystemToolbar: true)
    }

    /// 推送的状态和详情。类别由核心给（`PushStatus`、`ChatFailure`），壳这一段收不了推送的原因在
    /// `PushModel` 里，文字按类别写在这里。
    private var status: PushStatus {
        for section in settings.viewModel.sections {
            for entry in section.entries {
                if case let .notifications(_, status, _) = entry { return status }
            }
        }
        // 条目一直在设置里，取不到说明协议对不上，按没登记过显示。
        return .noEndpoint
    }

    private var line: String {
        switch status {
        case .registered:
            String(localized: .Settings.pushStatusRegistered)
        case .registering:
            String(localized: .Settings.pushStatusRegistering)
        case .noEndpoint:
            String(localized: .Settings.pushStatusNotSet)
        case .unavailable:
            push.unavailability?.line ?? String(localized: .Settings.pushStatusUnavailable)
        case let .failed(reason, _):
            switch reason {
            case .transportUnavailable: String(localized: .Settings.pushStatusFailedTransport)
            case .unauthorized: String(localized: .Settings.pushStatusFailedUnauthorized)
            case .forbidden: String(localized: .Settings.pushStatusFailedForbidden)
            case .upstream: String(localized: .Settings.pushStatusFailedUpstream)
            case .notReady: String(localized: .Settings.pushStatusFailedNotReady)
            case .invalidInput: String(localized: .Settings.pushStatusFailedInvalidInput)
            case .other: String(localized: .Settings.pushStatusFailedUpstream)
            }
        }
    }

    /// 核心记下的原始原因（详情收起，排查时看）；和主行一样的话不再重复。
    private var detail: String? {
        switch status {
        case let .unavailable(message):
            message == line ? nil : message
        case let .failed(_, message):
            message
        default:
            nil
        }
    }
}
