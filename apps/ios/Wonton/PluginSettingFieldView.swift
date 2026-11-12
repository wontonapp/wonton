import SwiftUI
import WontonClient

/// 单个设置项的草稿值；只在主人动过的项上生成提交值，没动的不动。
enum Draft: Equatable {
    case toggle(Bool)
    case text(String)
    case choice(Int)
    case multi([Int])
}

/// 控件下面的说明和问题；字段文案由核心给，固定文案进本地化。
struct SupportingText: View {
    let text: String
    let danger: Bool

    init(_ text: String, danger: Bool) {
        self.text = text
        self.danger = danger
    }
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(verbatim: text)
            .wontonText(WontonTheme.typography.supporting)
            .fixedSize(horizontal: false, vertical: true)
            .foregroundStyle(Color(danger ? colors.danger : colors.textSecondary))
    }
}

// MARK: 草稿与现值的共用取值工具（保存侧 PluginSettingsScreen 也用）

/// 现在存着的值；没设过时用默认值，都没有时为空。
func storedValue(of field: PluginSettingField) -> SettingValue? {
    field.value ?? field.default
}

func flag(of value: SettingValue?) -> Bool {
    if case let .flag(isOn) = value { return isOn }
    return false
}

func sameValue(_ left: SettingValue?, _ right: SettingValue?) -> Bool {
    switch (left, right) {
    case let (.flag(leftFlag), .flag(rightFlag)): leftFlag == rightFlag
    case let (.integer(leftNumber), .integer(rightNumber)): leftNumber == rightNumber
    case let (.decimal(leftText), .decimal(rightText)): leftText == rightText
    case let (.text(leftText), .text(rightText)): leftText == rightText
    case let (.texts(leftList), .texts(rightList)): leftList == rightList
    case (.other, .other): true
    default: false
    }
}

/// 控件或现值、默认值是比这个 App 新的核心给的：该项改不了，整份表单也不能提交。
func unknownField(_ field: PluginSettingField) -> Bool {
    if case .other = field.kind { return true }
    return isOther(field.value) || isOther(field.default)
}

func isOther(_ value: SettingValue?) -> Bool {
    guard let value else { return false }
    if case .other = value { return true }
    return false
}

/// 必填项缺值：动过的看草稿，没动过的看现值和默认值。核心也会拒，这里先挡。
func missing(_ field: PluginSettingField, drafts: [String: Draft], localErrors: [String: String]) -> Bool {
    guard field.required, localErrors[field.name] == nil else { return false }
    switch drafts[field.name] {
    case let .text(input):
        return input.isEmpty
    case let .multi(items):
        return items.isEmpty
    case nil:
        return storedValue(of: field) == nil
    default:
        return false
    }
}

// MARK: 单个设置项的渲染

/// 一个设置项的一节：标题、控件、说明和问题文案；草稿存在页面层，这里只读写自己这项的草稿。
/// 预检或保存被拒时核心按字段给的明细从 `overlay` 进来，盖过视图里带着的问题。
struct PluginSettingFieldView: View {
    let field: PluginSettingField
    var overlay: SettingsFieldProblem?
    @Binding var drafts: [String: Draft]
    @Binding var touched: Set<String>
    @Binding var localErrors: [String: String]

    var body: some View {
        DetailSection(title: field.title) {
            if unknownField(field) {
                SupportingText(String(localized: .Plugin0.pluginSettingsUpdatesApp), danger: false)
            } else {
                control
            }
            if let description = field.description {
                SupportingText(description, danger: false)
            }
            if let local = localErrors[field.name] {
                SupportingText(local, danger: true)
            }
            if let problem = overlay ?? field.problem {
                SupportingText(problem.message, danger: true)
            }
            if missing(field, drafts: drafts, localErrors: localErrors) {
                SupportingText(String(localized: .Plugin3.pluginSettingsRequired), danger: true)
            }
        }
    }

    @ViewBuilder private var control: some View {
        let editable = !unknownField(field)
        switch field.kind {
        case .toggle:
            Toggle(isOn: toggleBinding) {
                // 标题在节标题上，开关行不放重复的标签。
                Text(verbatim: "")
            }
            .toggleStyle(WontonToggleStyle())
            .disabled(!editable)
        case .integer, .decimal, .text:
            TextField(String(localized: .Plugin3.pluginSettingsEmptyPrompt), text: textBinding)
                .textFieldStyle(.plain)
                .keyboardType(keyboard)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .disabled(!editable)
                .wontonField()
        case let .choice(options, multiple):
            ForEach(Array(options.enumerated()), id: \.offset) { index, option in
                StepOptionRow(
                    title: option.title,
                    isSelected: isSelected(index)
                ) {
                    select(index: index, multiple: multiple)
                }
                .disabled(!editable)
            }
        case .other:
            EmptyView()
        }
    }

    private var keyboard: UIKeyboardType {
        switch field.kind {
        case .integer: .numberPad
        case .decimal: .decimalPad
        default: .default
        }
    }

    private var textBinding: Binding<String> {
        Binding(
            get: {
                if case let .text(text) = drafts[field.name] { return text }
                return ""
            },
            set: {
                drafts[field.name] = .text($0)
                touched.insert(field.name)
                localErrors[field.name] = nil
            }
        )
    }

    private var toggleBinding: Binding<Bool> {
        Binding(
            get: {
                if case let .toggle(isOn) = drafts[field.name] { return isOn }
                return flag(of: storedValue(of: field))
            },
            set: {
                drafts[field.name] = .toggle($0)
                touched.insert(field.name)
            }
        )
    }

    private func isSelected(_ index: Int) -> Bool {
        switch drafts[field.name] {
        case let .choice(chosen):
            return chosen == index
        case let .multi(chosen):
            return chosen.contains(index)
        case nil:
            guard case let .choice(options, multiple) = field.kind, !multiple else { return false }
            return options.firstIndex(where: { sameValue($0.value, storedValue(of: field)) }) == index
        default:
            return false
        }
    }

    private func select(index: Int, multiple: Bool) {
        touched.insert(field.name)
        localErrors[field.name] = nil
        if multiple, case let .multi(chosen) = drafts[field.name] {
            drafts[field.name] = .multi(chosen.contains(index) ? chosen.filter { $0 != index } : chosen + [index])
        } else {
            drafts[field.name] = multiple ? .multi([index]) : .choice(index)
        }
    }

    /// 多选的现值对应到选项下标；认不出的值不进草稿。
    static func chosenIndices(of value: SettingValue?, in field: PluginSettingField) -> [Int] {
        guard case let .texts(list) = value, case let .choice(options, _) = field.kind else { return [] }
        var indices: [Int] = []
        for item in list {
            let match = options.firstIndex { sameValue($0.value, .text(value: item)) }
            if let index = match, !indices.contains(index) {
                indices.append(index)
            }
        }
        return indices
    }
}
