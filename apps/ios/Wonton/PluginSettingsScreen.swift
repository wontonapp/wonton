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
                    PluginSettingFieldView(
                        field: field,
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
    }

    private func text(of value: SettingValue?) -> String {
        switch value {
        case let .integer(number): String(number)
        case let .decimal(decimal): decimal
        case let .text(text): text
        default: ""
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

    /// 必填项没值：动过的看草稿，没动过的看现值和默认值。核心也会拒，这里先挡。
    private func missingRequired(in form: SettingsForm) -> Bool {
        form.fields.contains { missing($0, drafts: drafts, localErrors: localErrors) }
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
            switch submitted(field: field, draft: draft, errors: &localErrors) {
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
