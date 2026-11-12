#if DEBUG
import SwiftUI
import WontonClient

/// #Preview（每种字段 + 有问题 + 保存中）：固定假数据，不连核心；标题说明按核心给的写法，壳里不拼。
private func previewField(
    name: String,
    heading: String,
    kind: PluginSettingKind,
    required: Bool = false,
    value: SettingValue? = nil,
    default defaultValue: SettingValue? = nil,
    problem: SettingsFieldProblem? = nil,
    services: [String] = []
) -> PluginSettingField {
    PluginSettingField(
        name: name,
        title: heading,
        description: nil,
        required: required,
        kind: kind,
        value: value,
        default: defaultValue,
        services: services,
        problem: problem
    )
}

private func previewOption(_ value: String, _ title: String) -> PluginSettingOption {
    PluginSettingOption(value: .text(value: value), title: title)
}

private func previewProblem(_ kind: SettingsFieldProblemKind, _ reason: String) -> SettingsFieldProblem {
    SettingsFieldProblem(kind: kind, message: reason)
}

/// 每种控件一项：开关、整数、小数、文本、单选、多选，和一项这个 App 还画不出来的。
private let previewFields: [PluginSettingField] = [
    previewField(
        name: "eula", heading: "Accept the Minecraft EULA", kind: .toggle,
        value: .flag(value: false), services: ["server"]
    ),
    previewField(
        name: "max_players", heading: "Max players", kind: .integer(min: 1, max: 100),
        value: .integer(value: 20)
    ),
    previewField(
        name: "backup_interval", heading: "Backup interval (hours)", kind: .decimal(min: "0.5", max: "24.0"),
        value: .decimal(value: "1.5")
    ),
    previewField(
        name: "motd", heading: "Message of the day", kind: .text(minLength: 1, maxLength: 40, pattern: nil),
        value: .text(value: "Play together")
    ),
    previewField(
        name: "difficulty", heading: "Difficulty",
        kind: .choice(
            options: [previewOption("peaceful", "Peaceful"), previewOption("normal", "Normal")],
            multiple: false
        ),
        value: .text(value: "normal")
    ),
    previewField(
        name: "modules", heading: "Enabled modules",
        kind: .choice(
            options: [previewOption("chat", "Chat"), previewOption("map", "Map"), previewOption("economy", "Economy")],
            multiple: true
        ),
        value: .texts(value: ["chat"])
    ),
    previewField(name: "new_kind", heading: "A setting this app can't render yet", kind: .other)
]

/// 有问题的一页：一项标红（核心给的话）、一项必填没填，外加画不出来的那项。
private let previewProblemFields: [PluginSettingField] = [
    previewField(
        name: "max_players", heading: "Max players", kind: .integer(min: 1, max: 100),
        value: .integer(value: 500),
        problem: previewProblem(.aboveMaximum, "Must be a number between 1 and 100")
    ),
    previewField(
        name: "motd", heading: "Message of the day", kind: .text(minLength: 1, maxLength: 40, pattern: nil),
        required: true
    ),
    previewField(name: "new_kind", heading: "A setting this app can't render yet", kind: .other)
]

/// 一页设置项：假草稿状态在预览自己手里。
private struct PluginSettingsFieldsPreview: View {
    let fields: [PluginSettingField]

    @State private var drafts: [String: Draft] = [:]
    @State private var touched: Set<String> = []
    @State private var localErrors: [String: String] = [:]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                ForEach(fields, id: \.name) { field in
                    PluginSettingFieldView(
                        field: field,
                        drafts: $drafts,
                        touched: $touched,
                        localErrors: $localErrors
                    )
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }
}

#Preview("Fields") {
    PluginSettingsFieldsPreview(fields: previewFields)
}

#Preview("Problems") {
    PluginSettingsFieldsPreview(fields: previewProblemFields)
}

#Preview("Saving") {
    PluginSettingsSaveTrailing(saving: true, blocked: false, onSave: {})
}
#endif
