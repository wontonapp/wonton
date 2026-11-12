import SwiftUI
import WontonClient

extension PluginTrust {
    var label: String {
        switch self {
        case .official: return String(localized: .pluginTrustOfficial)
        case .community, .local, .unknown: return String(localized: .pluginTrustUnreviewed)
        case .system: return String(localized: .pluginTrustSystem)
        }
    }

    func tint(_ colors: ThemeColors) -> Rgb {
        switch self {
        case .official: return colors.success
        case .community, .local, .unknown: return colors.warning
        case .system: return colors.textSecondary
        }
    }
}

extension PluginOperation {
    var runningText: String {
        switch self {
        case .install: return String(localized: .pluginTaskInstallRunning)
        case .upgrade: return String(localized: .pluginTaskUpgradeRunning)
        case .uninstall: return String(localized: .pluginTaskUninstallRunning)
        }
    }

    var doneText: String {
        switch self {
        case .install: return String(localized: .pluginTaskInstallDone)
        case .upgrade: return String(localized: .pluginTaskUpgradeDone)
        case .uninstall: return String(localized: .pluginTaskUninstallDone)
        }
    }
}

/// 来源、高权限、新增的权限这类小标签。
struct Tag: View {
    let text: String
    let tint: Rgb

    var body: some View {
        Text(text)
            .wontonText(WontonTheme.typography.caption)
            .padding(.horizontal, WontonTheme.spacing.s8)
            .foregroundStyle(Color(tint))
            .background(Color(tint).opacity(0.14), in: Capsule())
    }
}

/// 核心给的说明，`emphasis` 的醒目显示。
struct NoticeLines: View {
    let lines: [CardLine]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                    (line.emphasis ? WontonIcon.warningFill : WontonIcon.info).view(size: WontonTheme.size.iconSmall)
                    Text(line.text).wontonText(WontonTheme.typography.body)
                }
                .foregroundStyle(Color(line.emphasis ? colors.warning : colors.textSecondary))
            }
        }
    }
}

/// 要授予的一项权限：有风险级别时醒目，升级时新申请的、核心不支持的标出来。
struct PermissionRow: View {
    let line: PermissionLine
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let risky = line.risk != nil
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            (risky ? WontonIcon.warningFill : WontonIcon.shield)
                .view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(risky ? colors.warning : colors.textSecondary))
            Text(line.text)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(risky ? colors.warning : colors.textPrimary))
            Spacer(minLength: 0)
            if line.added { Tag(text: String(localized: .pluginPermissionAdded), tint: colors.primary) }
            if !line.supported { Tag(text: String(localized: .pluginPermissionUnsupported), tint: colors.danger) }
        }
    }
}

/// 详情和确认页里的一节。
struct DetailSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(title)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .accessibilityAddTraits(.isHeader)
            content
        }
    }
}
