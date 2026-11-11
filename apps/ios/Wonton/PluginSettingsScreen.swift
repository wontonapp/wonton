import SwiftUI
import WontonClient

/// 插件的原生设置页（ADR-0077）：表单形状由核心给，壳只渲染控件子集，把改动的值发回去。
/// 保存只在提交时校验：失败显示核心给的原因，并重读整页把字段问题标到控件上；不提供 JSON 编辑器。
struct PluginSettingsScreen: View {
    let model: PluginsModel
    let pluginId: String

    @State private var loaded: Result<PluginSettingsView, Error>?
    /// 控件的草稿值；只在主人动过的项上生成提交值，没动的不动。
    @State private var drafts: [String: Draft] = [:]
    @State private var touched: Set<String> = []
    /// 整数、小数没填成时就地提示，不把没填成的值发回去。
    @State private var localErrors: [String: String] = [:]
    @State private var saving = false
    @State private var problem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen(String(localized: .pluginSettingsTitle)) {
                if saving {
                    ProgressView().padding(.trailing, WontonTheme.spacing.s12)
                } else {
                    Button(String(localized: .commonSave)) {
                        Task { await save() }
                    }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(blocked)
                    .padding(.trailing, WontonTheme.spacing.s12)
                }
            }
    }

    private var view: PluginSettingsView? {
        try? loaded?.get()
    }

    @ViewBuilder private var content: some View {
        switch loaded {
        case nil:
            LoadingView()
        case let .failure(error):
            ProblemView(problem: error.problem) {
                Task { await load() }
            }
        case let .success(view):
            form(view)
        }
    }

    private func form(_ view: PluginSettingsView) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem {
                    ProblemNotice(problem: problem, onClose: { self.problem = nil })
                }
                if unknownValue(in: view.form) {
                    WontonNotice(icon: .warning, title: String(localized: .pluginSettingsUpdatesApp))
                }
                ForEach(view.form.fields, id: \.name) { field in
                    section(field)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .task(id: view.schemaDigest) { resetDrafts(view) }
    }

    // MARK: 控件

    @ViewBuilder private func section(_ field: PluginSettingField) -> some View {
        DetailSection(title: field.title) {
            if unknownField(field) {
                SupportingText(String(localized: .pluginSettingsUpdatesApp), danger: false)
            } else {
                control(field)
            }
            if let description = field.description {
                SupportingText(description, danger: false)
            }
            if let local = localErrors[field.name] {
                SupportingText(local, danger: true)
            }
            if let problem = field.problem {
                SupportingText(problem.message, danger: true)
            }
            if missing(field) {
                SupportingText(String(localized: .pluginSettingsRequired), danger: true)
            }
        }
    }

    @ViewBuilder private func control(_ field: PluginSettingField) -> some View {
        let editable = !unknownField(field)
        switch field.kind {
        case .toggle:
            Toggle(isOn: toggleBinding(field)) {
                // 标题在节标题上，开关行不放重复的标签。
                Text(verbatim: "")
            }
            .toggleStyle(WontonToggleStyle())
            .disabled(!editable)
        case .integer, .decimal, .text:
            TextField(String(localized: .pluginSettingsEmptyPrompt), text: textBinding(field))
                .textFieldStyle(.plain)
                .keyboardType(keyboard(field))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .disabled(!editable)
                .wontonField()
        case let .choice(options, multiple):
            ForEach(Array(options.enumerated()), id: \.offset) { index, option in
                StepOptionRow(
                    title: option.title,
                    isSelected: isSelected(field, index: index)
                ) {
                    select(field, index: index, multiple: multiple)
                }
                .disabled(!editable)
            }
        case .other:
            EmptyView()
        }
    }

    private func keyboard(_ field: PluginSettingField) -> UIKeyboardType {
        switch field.kind {
        case .integer: .numberPad
        case .decimal: .decimalPad
        default: .default
        }
    }

    // MARK: 草稿

    private enum Draft {
        case toggle(Bool)
        case text(String)
        case choice(Int)
        case multi([Int])
    }

    /// 打开和重读后按视图里的现值重建草稿；保存失败重读时不调它，主人填的还在。
    private func resetDrafts(_ view: PluginSettingsView) {
        var drafts: [String: Draft] = [:]
        for field in view.form.fields {
            let current = storedValue(of: field)
            switch field.kind {
            case .toggle:
                drafts[field.name] = .toggle(flag(of: current))
            case .integer, .decimal, .text:
                drafts[field.name] = .text(text(of: current))
            case .choice(_, true):
                drafts[field.name] = .multi(indices(of: current, in: field))
            case .choice, .other:
                break
            }
        }
        self.drafts = drafts
        touched = []
        localErrors = [:]
    }

    /// 现在存着的值；没设过时用默认值，都没有时为空。
    private func storedValue(of field: PluginSettingField) -> SettingValue? {
        field.value ?? field.default
    }

    private func flag(of value: SettingValue?) -> Bool {
        if case let .flag(isOn) = value { return isOn }
        return false
    }

    private func text(of value: SettingValue?) -> String {
        switch value {
        case let .integer(number): String(number)
        case let .decimal(decimal): decimal
        case let .text(text): text
        default: ""
        }
    }

    private func indices(of value: SettingValue?, in field: PluginSettingField) -> [Int] {
        guard case let .texts(list) = value, case let .choice(options, _) = field.kind else { return [] }
        var indices: [Int] = []
        for item in list {
            if let index = options.firstIndex(where: { sameValue($0.value, item) }), !indices.contains(index) {
                indices.append(index)
            }
        }
        return indices
    }

    private func sameValue(_ left: SettingValue?, _ right: SettingValue?) -> Bool {
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

    private func textBinding(_ field: PluginSettingField) -> Binding<String> {
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

    private func toggleBinding(_ field: PluginSettingField) -> Binding<Bool> {
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

    private func isSelected(_ field: PluginSettingField, index: Int) -> Bool {
        switch drafts[field.name] {
        case let .choice(chosen):
            chosen == index
        case let .multi(chosen):
            chosen.contains(index)
        case nil:
            guard case let .choice(options, multiple) = field.kind, !multiple else { return false }
            return options.firstIndex(where: { sameValue($0.value, storedValue(of: field)) }) == index
        default:
            return false
        }
    }

    private func select(_ field: PluginSettingField, index: Int, multiple: Bool) {
        touched.insert(field.name)
        localErrors[field.name] = nil
        if multiple, case let .multi(chosen) = drafts[field.name] {
            drafts[field.name] = .multi(chosen.contains(index) ? chosen.filter { $0 != index } : chosen + [index])
        } else {
            drafts[field.name] = multiple ? .multi([index]) : .choice(index)
        }
    }

    // MARK: 提交

    /// 比这个 App 新的核心给的控件或值类型：整份表单不让提交；必填项缺值也一样。
    private var blocked: Bool {
        guard let view else { return true }
        return saving
            || unknownValue(in: view.form)
            || missingRequired(in: view.form)
            || !localErrors.isEmpty
    }

    private func unknownValue(in form: SettingsForm) -> Bool {
        form.fields.contains { unknownField($0) }
    }

    /// 控件或现值、默认值是比这个 App 新的核心给的：该项改不了，整份表单也不能提交。
    private func unknownField(_ field: PluginSettingField) -> Bool {
        if case .other = field.kind { return true }
        return isOther(field.value) || isOther(field.default)
    }

    private func isOther(_ value: SettingValue?) -> Bool {
        guard let value else { return false }
        if case .other = value { return true }
        return false
    }

    private func missing(_ field: PluginSettingField) -> Bool {
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

    /// 必填项没值：动过的看草稿，没动过的看现值和默认值。核心也会拒，这里先挡。
    private func missingRequired(in form: SettingsForm) -> Bool {
        form.fields.contains { missing($0) }
    }

    private func save() async {
        guard let view, !blocked, !saving else { return }
        saving = true
        problem = nil
        localErrors = [:]
        var values: [String: SettingValue?] = [:]
        var valid = true
        for field in view.form.fields {
            guard touched.contains(field.name), let draft = drafts[field.name] else { continue }
            switch (field.kind, draft) {
            case let (.toggle, .toggle(isOn)):
                values[field.name] = .flag(value: isOn)
            case let (.integer, .text(input)):
                if input.isEmpty {
                    values[field.name] = .some(nil)
                } else if let integer = Int64(input) {
                    values[field.name] = .integer(value: integer)
                } else {
                    localErrors[field.name] = String(localized: .pluginSettingsInvalidInteger)
                    valid = false
                }
            case let (.decimal, .text(input)):
                if input.isEmpty {
                    values[field.name] = .some(nil)
                } else if Decimal(string: input) != nil {
                    values[field.name] = .decimal(value: input)
                } else {
                    localErrors[field.name] = String(localized: .pluginSettingsInvalidDecimal)
                    valid = false
                }
            case let (.text, .text(input)):
                values[field.name] = input.isEmpty ? .some(nil) : .text(value: input)
            case let (.choice(options, false), .choice(index)):
                values[field.name] = options[index].value
            case let (.choice(options, true), .multi(indices)):
                values[field.name] = indices.isEmpty ? .some(nil) : .texts(value: indices.map { options[$0].value })
            default:
                continue
            }
        }
        guard valid else {
            saving = false
            return
        }
        do {
            let updated = try await model.core.setPluginSettings(
                id: pluginId,
                update: PluginSettingsUpdate(schemaDigest: view.schemaDigest, values: values)
            )
            loaded = .success(updated)
            resetDrafts(updated)
        } catch {
            problem = error.problem
            // 校验失败的明细核心在下一次读里给（field.problem），重读整页把问题标到控件上；主人填的不动。
            if let refreshed = try? await model.core.pluginSettings(id: pluginId) {
                loaded = .success(refreshed)
            }
        }
        saving = false
    }

    private func load() async {
        loaded = nil
        do {
            loaded = .success(try await model.core.pluginSettings(id: pluginId))
        } catch {
            loaded = .failure(error)
        }
    }
}

/// 控件下面的说明和问题；字段文案由核心给，固定文案进本地化。
private struct SupportingText: View {
    let text: String
    let danger: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(verbatim: text)
            .wontonText(WontonTheme.typography.supporting)
            .fixedSize(horizontal: false, vertical: true)
            .foregroundStyle(Color(danger ? colors.danger : colors.textSecondary))
    }
}
