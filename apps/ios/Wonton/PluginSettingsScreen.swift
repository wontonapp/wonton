import SwiftUI
import WontonClient

/// 插件的原生设置页（ADR-0077）：表单形状由核心给，壳只渲染控件子集，把改动的值发回去。
/// 改动停下来就把候选值发给核心预检（防抖），问题标到控件旁；保存是最终一道门，失败显示核心给的原因。
/// 不提供 JSON 编辑器。
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
    /// 实时预检的最新结果；只拿来标问题，显示值照旧以本地草稿优先。
    @State private var previewed: PluginSettingsView?
    @State private var previewTask: Task<Void, Never>?
    /// 预检或保存被拒时核心按字段给的明细：键是属性名，盖过视图里带着的问题。
    @State private var fieldProblems: [String: SettingsFieldProblem] = [:]
    /// 保存成功后这次改动会不会重启应用：改动的项里有服务用到。
    @State private var restartHint = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen(String(localized: .Plugin1.pluginSettingsTitle), usesSystemToolbar: true) {
                PluginSettingsSaveTrailing(saving: saving, blocked: blocked) {
                    Task { await save() }
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
        // 预检结果盖在基础视图上：字段问题换成核心按候选值算的，控件的显示值仍以本地草稿优先。
        let displayed = previewed ?? view
        return ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if restartHint {
                    WontonNotice(
                        icon: .info,
                        title: String(localized: .Plugin2.pluginSettingsRestartHint),
                        isError: false)
                }
                if let problem {
                    ProblemNotice(problem: problem, onClose: { self.problem = nil })
                }
                if unknownValue(in: displayed.form) {
                    WontonNotice(icon: .warning, title: String(localized: .Plugin0.pluginSettingsUpdatesApp))
                }
                ForEach(displayed.form.fields, id: \.name) { field in
                    PluginSettingFieldView(
                        field: field,
                        overlay: fieldProblems[field.name],
                        drafts: $drafts,
                        touched: $touched,
                        localErrors: $localErrors
                    )
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.immediately)
        .task(id: view.schemaDigest) { resetDrafts(view) }
        .onChange(of: drafts) { schedulePreview() }
        .onDisappear { previewTask?.cancel() }
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
                drafts[field.name] = .multi(PluginSettingFieldView.chosenIndices(of: current, in: field))
            case .choice, .other:
                break
            }
        }
        self.drafts = drafts
        touched = []
        localErrors = [:]
        previewTask?.cancel()
        previewed = nil
        fieldProblems = [:]
    }

    private func text(of value: SettingValue?) -> String {
        switch value {
        case let .integer(number): String(number)
        case let .decimal(decimal): decimal
        case let .text(text): text
        default: ""
        }
    }

    // MARK: 实时预检

    /// 改动停下来才把候选值发给核心预检；没动过的项不发。拿不到就维持现状，保存时核心会再拦一次。
    private func schedulePreview() {
        previewTask?.cancel()
        guard !touched.isEmpty else {
            previewed = nil
            fieldProblems = [:]
            return
        }
        previewTask = Task {
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled, !touched.isEmpty, let view else { return }
            var scratch: [String: String] = [:]
            let (values, _) = candidateValues(view, errors: &scratch)
            do {
                previewed = try await model.core.previewPluginSettings(
                    id: pluginId,
                    update: PluginSettingsUpdate(schemaDigest: view.schemaDigest, values: values)
                )
                fieldProblems = [:]
            } catch let failure as CoreError {
                // 候选值没过校验：明细逐字段标红。
                if case let .SettingsInvalid(_, fields) = failure {
                    fieldProblems = Dictionary(
                        uniqueKeysWithValues: fields.map {
                            ($0.name, SettingsFieldProblem(kind: $0.kind, message: $0.message))
                        }
                    )
                }
            } catch {
                // 别的失败维持现状。
            }
        }
    }

    // MARK: 提交

    /// 比这个 App 新的核心给的控件或值类型：整份表单不让提交；必填项缺值和核心标出的问题也一样。
    private var blocked: Bool {
        guard let view else { return true }
        let displayed = previewed ?? view
        return saving
            || unknownValue(in: displayed.form)
            || missingRequired(in: displayed.form)
            || !localErrors.isEmpty
            || !fieldProblems.isEmpty
            || displayed.form.fields.contains { $0.problem != nil }
    }

    private func unknownValue(in form: SettingsForm) -> Bool {
        form.fields.contains { unknownField($0) }
    }

    /// 必填项没值：动过的看草稿，没动过的看现值和默认值。核心也会拒，这里先挡。
    private func missingRequired(in form: SettingsForm) -> Bool {
        form.fields.contains { missing($0, drafts: drafts, localErrors: localErrors) }
    }

    private func save() async {
        guard let view, !blocked, !saving else { return }
        saving = true
        problem = nil
        localErrors = [:]
        let (values, valid) = candidateValues(view, errors: &localErrors)
        guard valid else {
            saving = false
            return
        }
        do {
            let updated = try await model.core.setPluginSettings(
                id: pluginId,
                update: PluginSettingsUpdate(schemaDigest: view.schemaDigest, values: values)
            )
            restartHint = updated.form.fields.contains {
                values[$0.name] != nil && !$0.services.isEmpty
            }
            loaded = .success(updated)
            resetDrafts(updated)
        } catch let failure as CoreError {
            problem = failure.problem
            if case let .SettingsInvalid(_, fields) = failure {
                // 校验没过：明细逐字段标到控件旁，不用再问核心。
                fieldProblems = Dictionary(
                    uniqueKeysWithValues: fields.map {
                        ($0.name, SettingsFieldProblem(kind: $0.kind, message: $0.message))
                    }
                )
            } else {
                // 别的失败（如设置文件换过了）：重读整页拿最新状态；主人填的不动。
                if let refreshed = try? await model.core.pluginSettings(id: pluginId) {
                    loaded = .success(refreshed)
                }
            }
        } catch {
            problem = error.problem
        }
        saving = false
    }

    /// 动过的项换成的提交值；没填成的项不进表（`valid` 为 false，原因已在 `errors` 里就地标出）。
    private func candidateValues(
        _ view: PluginSettingsView,
        errors: inout [String: String]
    ) -> (values: [String: SettingValue?], valid: Bool) {
        var values: [String: SettingValue?] = [:]
        var valid = true
        for field in view.form.fields {
            guard touched.contains(field.name), let draft = drafts[field.name] else { continue }
            switch submitted(field: field, draft: draft, errors: &errors) {
            case let .set(value):
                values[field.name] = .some(value)
            case .clear:
                values[field.name] = .some(nil)
            case .invalid:
                valid = false
            case .skip:
                continue
            }
        }
        return (values, valid)
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

/// 栏上的保存按钮：进行中换成转圈，没法保存时置灰。
struct PluginSettingsSaveTrailing: View {
    let saving: Bool
    let blocked: Bool
    let onSave: () -> Void

    var body: some View {
        if saving {
            ProgressView().padding(.trailing, WontonTheme.spacing.s12)
        } else {
            Button(String(localized: .Common.commonSave)) { onSave() }
                .wontonBarAction()
                .disabled(blocked)
                .padding(.trailing, WontonTheme.spacing.s12)
        }
    }
}
