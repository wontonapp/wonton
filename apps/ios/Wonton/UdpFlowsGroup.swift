import SwiftUI
import WontonClient

struct UdpFlowsGroup: View {
    let flows: [UdpFlowStats]
    let evictedDatagrams: UInt64?
    let installed: InstalledPlugins?
    @State private var expanded = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        WontonGroup {
            Button { expanded.toggle() } label: {
                HStack(spacing: WontonTheme.spacing.s12) {
                    Text(.Connection2.connectionUdpFlows(count: flows.count))
                        .foregroundStyle(Color(colors.textPrimary))
                    Spacer(minLength: 0)
                    if let evictedDatagrams, evictedDatagrams > 0 {
                        Text(.Connection1.connectionUdpFlowDropped(evictedDatagrams.formatted()))
                            .foregroundStyle(Color(colors.textSecondary))
                            .multilineTextAlignment(.trailing)
                            .accessibilityIdentifier("udpBufferEvictedDatagrams")
                    }
                    WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        .accessibilityHidden(true)
                }
                .wontonText(WontonTheme.typography.body)
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.vertical, WontonTheme.spacing.s12)
                .frame(minHeight: WontonTheme.size.target)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("udpFlowsSummary")
            .accessibilityValue(
                expanded ? Text(.Appearance1.appearanceExpanded) : Text(.Appearance0.appearanceCollapsed)
            )
            if expanded {
                ForEach(flows, id: \.flowId) { flow in
                    WontonSeparator()
                    WontonRow(
                        title: installed?.plugins.first { $0.id == flow.pluginId }?.title ?? flow.pluginId,
                        value: traffic(flow)
                    )
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("udpFlow-\(flow.flowId)")
                }
            }
        }
    }

    private func traffic(_ flow: UdpFlowStats) -> String {
        if flow.oversizedDrops > 0 {
            return String(localized: .Connection2.connectionUdpFlowTrafficDropped(
                String(flow.port), flow.receivedPackets.formatted(), flow.sentPackets.formatted(),
                flow.oversizedDrops.formatted()
            ))
        }
        return String(localized: .Connection2.connectionUdpFlowTraffic(
            String(flow.port), flow.receivedPackets.formatted(), flow.sentPackets.formatted()
        ))
    }
}
