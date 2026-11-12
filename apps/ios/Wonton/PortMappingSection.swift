import SwiftUI
import WontonClient

/// 插件声明的映射端口（ADR-0072）：iOS 不走系统通道，App 在前台时自己用；这里如实列出，没有开关。
struct PortMappingSection: View {
    let installed: InstalledPlugin

    var body: some View {
        DetailSection(title: String(localized: .portMappingTitle)) {
            WontonGroup {
                ForEach(Array(installed.tunnelPorts.enumerated()), id: \.offset) { index, port in
                    if index > 0 { WontonSeparator() }
                    WontonRow(title: String(localized: .portMappingPortLine(
                        Int(port.port), protocolName(port.protocol))))
                }
            }
        }
    }

    /// 协议名：TCP 和 UDP 是协议名，不翻译。
    private func protocolName(_ protocol: PortProtocol) -> String {
        switch `protocol` {
        case .tcp: "TCP"
        case .udp: "UDP"
        case .other: String(localized: .portMappingProtocolOther)
        }
    }
}
