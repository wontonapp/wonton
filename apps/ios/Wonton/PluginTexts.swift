import SwiftUI
import WontonClient

extension StoreFailure {
    var text: String {
        switch self {
        case .unreachable: return "连不上 GitHub，列表可能不是最新的"
        case .timeout: return "GitHub 太慢，没有及时答完，列表可能不是最新的"
        case .rateLimited: return "GitHub 限速了，过几分钟再试"
        case .invalidIndex: return "商店的列表读不懂，稍后再试"
        case .indexTooNew: return "商店的列表比家里的服务器新，先升级家里的服务器"
        case .upstream: return "商店的来源出了问题，稍后再试"
        case .incomplete: return "有的没读到，列表不完整"
        case .disabled: return "社区应用已关闭"
        case .other: return "商店出了问题，稍后再试"
        }
    }
}

extension PluginFailure {
    /// 失败以后主人下一步该做什么，跟在核心写的原因后面。
    var nextStep: String {
        switch self {
        case .transportUnavailable: return "连上家里的服务器后再试。"
        case .unauthorized: return "家里的服务器不认这台设备，请重新扫码配对。"
        case .sourceUnavailable: return "家里的服务器连不上应用的来源，稍后再试。"
        case .notFound: return "来源里没有这个应用或版本，刷新商店后再试。"
        case .changed: return "应用在确认之后变了，请重新查看、确认。"
        case .unsupported: return "家里的服务器不支持这个应用，先升级家里的服务器。"
        case .conflict: return "它已经装了，或者正在安装、升级、卸载，等那边做完。"
        case .invalid: return "应用包不合法，请联系应用的作者。"
        case .lanUnavailable: return "在局域网设置里选好网卡或地址后再试。"
        case .coreUpgrading: return "家里的服务器正在升级，升级结束后再试。"
        case .egressProxyUnavailable: return "检查家里服务器上的代理，或者在出网代理设置里改地址后再试。"
        case .upstream: return "家里的服务器出了错，稍后再试。"
        case .protocol: return "App 和家里的服务器版本对不上，请更新 App 或家里的服务器。"
        }
    }
}

extension PluginTrust {
    var label: String {
        switch self {
        case .official: return "已审核"
        case .community, .unknown: return "未审核"
        case .local: return "本地"
        case .system: return "系统"
        }
    }

    func tint(_ colors: ThemeColors) -> Rgb {
        switch self {
        case .official: return colors.success
        case .community, .unknown: return colors.warning
        case .local, .system: return colors.textSecondary
        }
    }
}

extension PluginOperation {
    var runningText: String {
        switch self {
        case .install: return "正在安装"
        case .upgrade: return "正在升级"
        case .uninstall: return "正在卸载"
        }
    }

    var doneText: String {
        switch self {
        case .install: return "已安装"
        case .upgrade: return "已升级"
        case .uninstall: return "已卸载"
        }
    }

    var failedText: String {
        switch self {
        case .install: return "安装失败"
        case .upgrade: return "升级失败"
        case .uninstall: return "卸载失败"
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

/// 取列表、详情或预览失败，整页显示原因和下一步。
struct ProblemView: View {
    let message: String
    let nextStep: String?
    let retry: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.warning.view().foregroundStyle(Color(colors.danger))
            Text(message)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if let nextStep {
                Text(nextStep)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            Button("重试", action: retry)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .padding(.top, WontonTheme.spacing.s8)
        }
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
    }
}

/// 列表还在，但最近一次刷新出了问题：放在列表上方。
struct ProblemRow: View {
    let message: String
    let nextStep: String?

    var body: some View {
        WontonNotice(icon: .warning, title: message, lines: nextStep.map { [$0] } ?? [])
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

/// 要授予的一项权限：高权限醒目，升级时新申请的、核心不支持的标出来。
struct PermissionRow: View {
    let line: PermissionLine
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            (line.high ? WontonIcon.warningFill : WontonIcon.shield)
                .view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(line.high ? colors.warning : colors.textSecondary))
            Text(line.text)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(line.high ? colors.warning : colors.textPrimary))
            Spacer(minLength: 0)
            if line.added { Tag(text: "新增", tint: colors.primary) }
            if !line.supported { Tag(text: "不支持", tint: colors.danger) }
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
