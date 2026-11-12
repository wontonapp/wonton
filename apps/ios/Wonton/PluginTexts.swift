import SwiftUI
import WontonClient

extension PluginTrust {
    var label: String {
        switch self {
        case .official: return String(localized: .Plugin0.pluginTrustOfficial)
        case .community, .local, .unknown: return String(localized: .Plugin3.pluginTrustUnreviewed)
        case .system: return String(localized: .Plugin0.pluginTrustSystem)
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
        case .install: return String(localized: .Plugin2.pluginTaskInstallRunning)
        case .upgrade: return String(localized: .Plugin2.pluginTaskUpgradeRunning)
        case .uninstall: return String(localized: .Plugin2.pluginTaskUninstallRunning)
        }
    }

    var doneText: String {
        switch self {
        case .install: return String(localized: .Plugin1.pluginTaskInstallDone)
        case .upgrade: return String(localized: .Plugin0.pluginTaskUpgradeDone)
        case .uninstall: return String(localized: .Plugin3.pluginTaskUninstallDone)
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

/// 权限说明和开关共用同一排版；标签放在独立行，避免大字号时挤压开关。
struct PermissionRow: View {
    let line: PermissionLine
    var selection: Binding<Bool>?
    var busy = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            if let selection {
                Toggle(isOn: selection) { heading }
                    .toggleStyle(WontonToggleStyle())
                    .disabled(busy || !line.supported)
            } else {
                heading
            }
            labels
            if let notice = line.notice { supporting(notice) }
            if let offLoss = line.offLoss { supporting(offLoss) }
            if busy { ProgressView() }
        }
    }

    private var heading: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            (line.risk != nil ? WontonIcon.warningFill : WontonIcon.shield)
                .view(size: WontonTheme.size.iconSmall)
                .accessibilityHidden(true)
            Text(line.text)
                .wontonText(WontonTheme.typography.body)
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(Color(line.risk != nil ? colors.warning : colors.textPrimary))
    }

    private var labels: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            if let risk = line.riskLabel { Tag(text: risk, tint: colors.warning) }
            if line.optional { Tag(text: String(localized: .Plugin3.pluginPermissionOptional),
                tint: colors.textSecondary) }
            if line.added { Tag(text: String(localized: .Plugin3.pluginPermissionAdded), tint: colors.primary) }
            if !line.supported { Tag(text: String(localized: .Plugin0.pluginPermissionUnsupported),
                tint: colors.danger) }
        }
    }

    private func supporting(_ text: String) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
            .fixedSize(horizontal: false, vertical: true)
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
