import SwiftUI
import WontonClient

/// 插件声明的端口的本机映射（ADR-0072）：开关只管映射开不开，结果类别由核心给，这里只渲染。
/// iOS 切走后监听就停、别的 App 连不进来，写在开关下面，不给第三方 App 用的开关。
struct PortMappingSection: View {
    let model: PluginsModel
    let installed: InstalledPlugin
    @State private var problem: Problem?
    @Environment(\.wontonColors) private var colors

    private var mapping: PluginPortMapping? {
        model.viewModel.portMappings.first { $0.pluginId == installed.id }
    }

    /// 核心还没发来映射时按清单声明的列出来。
    private var ports: [MappedPort] { mapping?.ports ?? installed.tunnelPorts }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            WontonGroup {
                Toggle(isOn: Binding(get: { mapping?.enabled ?? false }, set: { change(to: $0) })) {
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(.portMappingTitle)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                        Text(.portMappingForegroundOnly)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                .toggleStyle(WontonToggleStyle())
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.vertical, WontonTheme.spacing.s12)
                ForEach(Array(ports.enumerated()), id: \.offset) { _, port in
                    WontonSeparator()
                    WontonRow(title: "127.0.0.1:\(port.port)", value: protocolLabel(port.protocol))
                }
            }
            if let state = mapping?.state, let reason = reason(state) {
                WontonNotice(icon: .warning, title: reason)
            }
            if let problem { ProblemNotice(problem: problem) }
        }
    }

    /// TCP 和 UDP 是协议名，不翻译。
    private func protocolLabel(_ value: PortProtocol) -> String {
        switch value {
        case .tcp: "TCP"
        case .udp: "UDP"
        case .other: String(localized: .portMappingProtocolOther)
        }
    }

    private func change(to enabled: Bool) {
        problem = nil
        do {
            try model.core.dispatch(action: .setPortMapping(pluginId: installed.id, enabled: enabled))
        } catch {
            problem = error.problem
        }
    }

    /// 没映射成功的原因；开着和关着不用解释。
    func reason(_ state: PortMappingState) -> String? {
        switch state {
        case .off, .on: return nil
        case .portInUse: return String(localized: .portMappingReasonPortInUse)
        case .serverTooOld: return String(localized: .portMappingReasonServerTooOld)
        case .tunnelDisconnected: return String(localized: .portMappingReasonTunnelDisconnected)
        case .pluginNotInstalled: return String(localized: .portMappingReasonPluginNotInstalled)
        case .portNotDeclared: return String(localized: .portMappingReasonPortNotDeclared)
        case .pluginNotRunning: return String(localized: .portMappingReasonPluginNotRunning)
        case .pluginUnreachable: return String(localized: .portMappingReasonPluginUnreachable)
        case .other: return String(localized: .portMappingReasonOther)
        }
    }
}
