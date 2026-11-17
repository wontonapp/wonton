import SwiftUI
import WontonClient

struct InstallPermissions: View {
    let permissions: [PermissionLine]
    let notices: [CardLine]

    var body: some View {
        if !notices.isEmpty { NoticeLines(lines: notices) }
        if !permissions.isEmpty {
            DetailSection(title: String(localized: .Install.installPermissions)) {
                ForEach(permissions, id: \.permission) { PermissionRow(line: $0) }
            }
        }
    }
}

/// 插件的网页在房间里当 Matrix widget 时能做什么（ADR-0096）：核心给原始能力串和级别，这里按级别各写一行，
/// 会碰到聊天内容的那一级醒目显示；原始能力串收在默认收起的“详情”里。
struct WidgetCapabilityRows: View {
    let capabilities: [WidgetCapability]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        // 高的在前。比这个 App 新的级别（`other`）按高的处理。
        row(capabilities.filter { $0.level != .normal }, high: true)
        row(capabilities.filter { $0.level == .normal }, high: false)
    }

    @ViewBuilder private func row(_ lines: [WidgetCapability], high: Bool) -> some View {
        if !lines.isEmpty {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                    (high ? WontonIcon.warningFill : WontonIcon.shield)
                        .view(size: WontonTheme.size.iconSmall)
                    Text(high ? String(localized: .Plugin1.pluginWidgetHigh)
                         : String(localized: .Plugin3.pluginWidgetNormal))
                        .wontonText(WontonTheme.typography.body)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .foregroundStyle(Color(high ? colors.warning : colors.textPrimary))
                if lines.contains(where: \.added) {
                    Tag(text: String(localized: .Plugin3.pluginPermissionAdded), tint: colors.primary)
                }
                ProblemDetail(text: lines.map(\.capability).joined(separator: "\n"))
            }
            .accessibilityIdentifier(high ? "widgetCapabilityHigh" : "widgetCapabilityNormal")
        }
    }
}
