import SwiftUI
import WontonClient

/// iOS 只在前台自动映射，不提供给第三方 App 用的手动开关（ADR-0072）。
struct PortMappingSection: View {
    let declaredPorts: [MappedPort]
    let mapping: PluginPortMapping?

    var body: some View {
        DetailSection(title: String(localized: .Plugins.portMappingTitle)) {
            WontonGroup {
                if let status {
                    WontonRow(title: status)
                        .accessibilityIdentifier("portMappingState")
                }
                ForEach(Array((mapping?.ports ?? declaredPorts).enumerated()), id: \.offset) { index, port in
                    if index > 0 || status != nil { WontonSeparator() }
                    WontonRow(title: String(localized: .Plugins.portMappingPortLine(
                        Int(port.port), protocolName(port.protocol))))
                }
            }
            if let reason {
                WontonNotice(icon: .warning, title: reason)
                    .accessibilityIdentifier("portMappingReason")
            }
            if mapping?.warning == .publicRelayBandwidth {
                WontonNotice(
                    icon: .warning,
                    title: String(localized: .Plugins.portMappingWarningPublicRelayBandwidth),
                    isError: false
                )
            }
        }
    }

    private var status: String? {
        switch mapping?.state {
        case .on: String(localized: .Design.designSwitchOn)
        case .off: String(localized: .Design.designSwitchOff)
        case nil: String(localized: .Common.commonUnknown)
        case .portInUse, .serverTooOld, .tunnelDisconnected, .pluginNotInstalled,
             .portNotDeclared, .pluginNotRunning, .pluginUnreachable, .other: nil
        }
    }

    private var reason: String? {
        switch mapping?.state {
        case nil, .on, .off: nil
        case .portInUse: String(localized: .Plugins.portMappingReasonPortInUse)
        case .serverTooOld: String(localized: .Plugins.portMappingReasonServerTooOld)
        case .tunnelDisconnected: String(localized: .Plugins.portMappingReasonTunnelDisconnected)
        case .pluginNotInstalled: String(localized: .Plugins.portMappingReasonPluginNotInstalled)
        case .portNotDeclared: String(localized: .Plugins.portMappingReasonPortNotDeclared)
        case .pluginNotRunning: String(localized: .Plugins.portMappingReasonPluginNotRunning)
        case .pluginUnreachable: String(localized: .Plugins.portMappingReasonPluginUnreachable)
        case .other: String(localized: .Plugins.portMappingReasonOther)
        }
    }

    /// 协议名：TCP 和 UDP 是协议名，不翻译。
    private func protocolName(_ protocol: PortProtocol) -> String {
        switch `protocol` {
        case .tcp: "TCP"
        case .udp: "UDP"
        case .other: String(localized: .Plugins.portMappingProtocolOther)
        }
    }
}
