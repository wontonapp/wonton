import SwiftUI
import WontonClient

struct PluginGrantedDevicesSection: View {
    let devices: [PluginGrantedDevice]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Plugin3.pluginGrantedDevices)) {
            ForEach(Array(devices.enumerated()), id: \.offset) { index, device in
                row(device)
                    .accessibilityIdentifier("grantedDevice-\(index)")
            }
        }
    }

    private func row(_ device: PluginGrantedDevice) -> some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            icon(device.kind).view(size: WontonTheme.size.icon)
                .foregroundStyle(Color(colors.textSecondary))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(verbatim: title(device))
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(device.available ? colors.textPrimary : colors.textSecondary))
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("grantedDeviceName")
                if !device.available {
                    status(String(localized: .Plugin3.pluginDeviceUnavailable), warning: false)
                        .accessibilityIdentifier("grantedDeviceUnavailable")
                }
                if device.system {
                    status(String(localized: .Install.installDeviceSystemDisk), warning: true)
                        .accessibilityIdentifier("grantedDeviceSystemDisk")
                }
                ProblemDetail(text: device.path, minimumTargetHeight: WontonTheme.size.target)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("grantedDeviceDetails")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
    }

    private func status(_ text: String, warning: Bool) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(warning ? colors.danger : colors.textSecondary))
            .fixedSize(horizontal: false, vertical: true)
    }

    private func title(_ device: PluginGrantedDevice) -> String {
        if let name = device.name, !name.isEmpty { return name }
        switch device.kind {
        case .serial: return String(localized: .Plugin2.pluginDeviceSerial)
        case .usb: return String(localized: .Install.installDeviceUsbAll)
        case .disk: return String(localized: .Plugin1.pluginDeviceDisk)
        case .other: return String(localized: .Plugin0.pluginDeviceOther)
        }
    }

    private func icon(_ kind: PluginDeviceKind) -> WontonIcon {
        switch kind {
        case .serial: return .serialPort
        case .usb: return .usb
        case .disk: return .disk
        case .other: return .device
        }
    }
}
