import SwiftUI
import WontonClient

extension PluginCategory {
    var name: String {
        switch self {
        case .ai: return "AI"
        case .agent: return "Agent"
        case .photos: return "照片"
        case .media: return "影音"
        case .files: return "文件"
        case .documents: return "文档"
        case .productivity: return "效率"
        case .communication: return "通讯"
        case .home: return "智能家居"
        case .network: return "网络"
        case .security: return "安全"
        case .developer: return "开发"
        case .downloads: return "下载"
        case .tools: return "工具"
        case .system: return "系统"
        case .other: return "其他"
        }
    }
}

extension PluginFailure {
    /// 失败以后主人下一步该做什么，跟在核心写的原因后面。
    var nextStep: String {
        switch self {
        case .transportUnavailable: return "连上家里的服务器后再试。"
        case .unauthorized: return "家里的服务器不认这台设备，请重新扫码配对。"
        case .sourceUnavailable: return "家里的服务器连不上插件的来源，稍后再试。"
        case .notFound: return "来源里没有这个插件或版本，刷新商店后再试。"
        case .changed: return "插件在确认之后变了，请重新查看、确认。"
        case .unsupported: return "家里的服务器不支持这个插件，先升级家里的服务器。"
        case .conflict: return "它已经装了，或者正在安装、升级、卸载，等那边做完。"
        case .invalid: return "插件包不合法，请联系插件的作者。"
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

    var tint: Color {
        switch self {
        case .official: return .blue
        case .community, .unknown: return .orange
        case .local, .system: return .gray
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
    let tint: Color

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .foregroundStyle(tint)
            .background(tint.opacity(0.14), in: Capsule())
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

    var body: some View {
        ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
            Label(line.text, systemImage: line.emphasis ? "exclamationmark.triangle.fill" : "info.circle")
                .font(line.emphasis ? .callout.weight(.semibold) : .callout)
                .foregroundStyle(line.emphasis ? Color.orange : Color.secondary)
        }
    }
}

/// 要授予的一项权限：高权限醒目，升级时新申请的、核心不支持的标出来。
struct PermissionRow: View {
    let line: PermissionLine

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: line.high ? "exclamationmark.shield.fill" : "checkmark.shield")
                .foregroundStyle(line.high ? Color.orange : Color.secondary)
            Text(line.text).fontWeight(line.high ? .semibold : .regular)
            Spacer(minLength: 4)
            if line.added { Tag(text: "新增", tint: .blue) }
            if !line.supported { Tag(text: "不支持", tint: .red) }
        }
    }
}
