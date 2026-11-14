import SwiftUI
import WontonClient

/// 确认页和确认卡片上的“设备”一节（ADR-0036）：串口每个服务选一个，USB 整类授权，硬盘勾选、系统盘有标记。
/// 怎么分组、点一项以后谁选中由核心定（`deviceChoices`、`toggleDevice`），这里只画；原始的设备 ID 和容器里的路径
/// 收在“详情”里。
struct DeviceChoicesSection: View {
    let choices: DeviceChoices
    let toggle: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Install.installDevices)) {
            ForEach(Array(choices.serial.enumerated()), id: \.offset) { _, group in
                note(String(localized: .Install.installDeviceSerialFor(group.service)))
                ForEach(group.devices, id: \.id) { row($0) }
            }
            if choices.serialMissing {
                note(String(localized: .Install.installDeviceSerialNone))
            }
            if choices.usb {
                Text(.Install.installDeviceUsbAll)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .accessibilityIdentifier("deviceUsbAll")
            }
            ForEach(choices.disks, id: \.id) { row($0) }
            if choices.disksMissing {
                note(String(localized: .Install.installDeviceDisksNone))
            }
            ForEach(choices.unknown, id: \.id) { device in
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    Text(verbatim: device.name)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    note(String(localized: .Install.installDeviceUpdateApp))
                }
            }
            if !raw.isEmpty { ProblemDetail(text: raw) }
        }
    }

    private func row(_ device: PluginDevice) -> some View {
        StepOptionRow(title: device.name, isSelected: device.chosen, select: { toggle(device.id) }, trailing: {
            if device.system {
                Tag(text: String(localized: .Install.installDeviceSystemDisk), tint: colors.danger)
                    .accessibilityIdentifier("deviceSystemDisk")
            }
        })
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("device-\(device.id)")
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
            .fixedSize(horizontal: false, vertical: true)
    }

    /// 排查时才看的原始值：核心认设备的 ID、交给哪个服务、在容器里的路径。
    private var raw: String {
        (choices.serial.flatMap(\.devices) + choices.disks + choices.unknown)
            .map { device in
                [device.name, device.id, [device.service, device.target].filter { !$0.isEmpty }.joined(separator: " ")]
                    .joined(separator: "\n")
            }
            .joined(separator: "\n\n")
    }
}

extension InstallPreview {
    /// 主人在确认页上开关过可选权限之后的权限项：`enabled` 是现在的选择。
    func permissionLines(enabling enabled: Set<String>) -> [PermissionLine] {
        permissions.map { line in
            guard line.optional else { return line }
            var line = line
            line.enabled = enabled.contains(line.permission)
            return line
        }
    }
}
