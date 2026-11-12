import SwiftUI
import UIKit
import WontonClient

/// 设置里的 App 锁（ADR-0102）：开关和离开后多久要求解锁。
/// 设备没有设备密码时开不了，如实说明并指去系统设置；设备密码后来被关掉时，锁页同样停在那里等。
struct AppLockSettingsView: View {
    let model: AppLockModel
    @State private var needsDeviceLock = false
    @Environment(\.wontonColors) private var colors

    private var viewModel: AppLockViewModel { model.viewModel }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    Toggle(isOn: Binding(
                        get: { viewModel.enabled },
                        set: { model.set(enabled: $0, timeout: viewModel.timeout) }
                    )) {
                        Text(.appLockTitle)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                    .disabled(needsDeviceLock && !viewModel.enabled)
                    .toggleStyle(WontonToggleStyle())
                    .padding(WontonTheme.spacing.s16)
                }
                if viewModel.enabled {
                    timeoutChoices
                } else if needsDeviceLock {
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                        Text(.appLockNeedDeviceLock)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                        Button(action: openPasscodeSettings) {
                            Text(.appLockOpenSettings).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                }
                if let problem = viewModel.problem {
                    Text(problem)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .task { needsDeviceLock = !SystemAuth.canAuthenticate() }
        .wontonDialog(problem: model.rejection, isPresented: Binding(
            get: { model.rejection != nil }, set: { if !$0 { model.clearRejection() } }
        ))
    }

    private var timeoutChoices: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.appLockTimeout)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .padding(.horizontal, WontonTheme.spacing.s16)
            WontonGroup {
                ForEach(Array(AppLockTimeout.all.enumerated()), id: \.offset) { index, timeout in
                    if index > 0 { WontonSeparator() }
                    TimeoutRow(timeout: timeout, isSelected: timeout == viewModel.timeout) {
                        model.set(enabled: true, timeout: timeout)
                    }
                }
            }
        }
    }
}

private struct TimeoutRow: View {
    let timeout: AppLockTimeout
    let isSelected: Bool
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(timeout.label)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                Spacer(minLength: 0)
                if isSelected {
                    WontonIcon.check.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.accentInk))
                }
            }
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

extension AppLockTimeout {
    /// 按设置页里的显示顺序。
    static let all: [AppLockTimeout] = [.immediate, .oneMinute, .fiveMinutes, .fifteenMinutes]

    /// 档位文字，默认档兜住新核心多给的取值。
    var label: String {
        switch self {
        case .immediate: String(localized: .appLockTimeoutImmediate)
        case .oneMinute: String(localized: .appLockTimeoutOneMinute)
        case .fiveMinutes: String(localized: .appLockTimeoutFiveMinutes)
        case .fifteenMinutes: String(localized: .appLockTimeoutFifteenMinutes)
        default: String(localized: .appLockTimeoutImmediate)
        }
    }
}

/// 系统没有公开的设备密码设置地址；`App-Prefs` 打不开时退回本 App 的设置页。
func openPasscodeSettings() {
    if let url = URL(string: "App-Prefs:PASSCODE"), UIApplication.shared.canOpenURL(url) {
        UIApplication.shared.open(url)
    } else if let url = URL(string: UIApplication.openSettingsURLString) {
        UIApplication.shared.open(url)
    }
}
