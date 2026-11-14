import SwiftUI
import WontonClient

/// 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。
struct ConfirmationCardView: View {
    let view: ConfirmationView
    let answer: (CardChoice, [String: String], [String: SettingValue]) -> Void
    let stopWindow: () -> Void
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins
    @State private var showsPages = false
    @State private var secrets: [String: String] = [:]
    @State private var shownSecrets = false
    @State private var drafts: [String: Draft] = [:]
    @State private var touched: Set<String> = []
    @State private var localErrors: [String: String] = [:]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(view.card.title).wontonText(WontonTheme.typography.itemTitle)
            ForEach(Array(view.card.lines.enumerated()), id: \.offset) { _, line in
                Text(line.text)
                    .wontonText(WontonTheme.typography.supporting)
                    .fontWeight(line.emphasis ? .semibold : .regular)
                    .foregroundStyle(line.emphasis ? Color(colors.danger) : Color(colors.textPrimary))
            }
            if case let .toolCall(tool) = view.card.category {
                toolDetails(tool)
            }
            if !view.card.secretInputs.isEmpty {
                SecretInputsSection(
                    inputs: view.card.secretInputs, values: secrets,
                    enabled: view.card.state == .pending,
                    onShownChange: { shownSecrets = $0 },
                    onChange: { name, value in secrets[name] = value }
                )
            }
            if let form = view.card.settings {
                ForEach(form.fields, id: \.name) { field in
                    PluginSettingFieldView(field: field, overlay: nil, drafts: $drafts,
                        touched: $touched, localErrors: $localErrors)
                }
                .disabled(view.card.state != .pending || view.sending != nil)
            }
            answeredByLine
            state
            if let window = view.window {
                self.window(window)
            }
            if let problem = view.problem {
                note(problem.message, icon: .warning, color: colors.danger)
            }
            if let installed {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Button(TabText.open) { pages.open(installed.tabChoices[0].tab) }
                        .buttonStyle(WontonSmallButtonStyle())
                    if !added {
                        Button(TabText.addToMyApps) {
                            if installed.tabChoices.count > 1 {
                                showsPages = true
                            } else {
                                add(installed.tabChoices[0].tab)
                            }
                        }
                        .buttonStyle(WontonSmallButtonStyle())
                    }
                }
                .wontonSheet(
                    isPresented: $showsPages, title: installed.title, usesSystemPresentation: true
                ) { _ in
                    MyAppChoiceSheet(plugin: installed) { showsPages = false }
                        .padding(WontonTheme.spacing.pageInset)
                }
            }
            if !view.card.choices.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: WontonTheme.spacing.s8) { buttons }
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) { buttons }
                }
            }
        }
        // 时间线第一次量这一行时给的高度可能不够，主人没读完不该按确认。
        .fixedSize(horizontal: false, vertical: true)
        // 有密钥、点过“显示”或框里有值、卡片还没答完时，这一块在快照里要盖住。
        .secretSnapshotMask(view.card.state == .pending && hidesValues)
        .task(id: view.card.id) { initializeSettings() }
    }

    /// 有密钥、点过“显示”或框里有值、卡片还没答完。
    private var hidesValues: Bool {
        !view.card.secretInputs.isEmpty
            && (shownSecrets || view.card.secretInputs.contains { !(secrets[$0.name] ?? "").isEmpty })
    }

    private func toolDetails(_ tool: ConfirmationTool) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.ConfirmationCard.confirmationCardToolSource(tool.pluginTitle))
                .foregroundStyle(Color(colors.textSecondary))
            switch tool.impact {
            case .readOnly:
                EmptyView()
            case .changesData:
                Text(.ConfirmationCard.confirmationCardToolChangesData)
                    .foregroundStyle(Color(colors.warning))
            case let .destructive(target):
                Group {
                    Text(.ConfirmationCard.confirmationCardToolDestructive)
                    if let target {
                        Text(.ConfirmationCard.confirmationCardToolDestructiveTarget(target))
                    } else {
                        Text(.ConfirmationCard.confirmationCardToolDestructiveUnknown)
                    }
                }
                .foregroundStyle(Color(colors.danger))
            case .other:
                Text(.ConfirmationCard.confirmationCardToolUnknownImpact)
                    .foregroundStyle(Color(colors.warning))
            }
            if tool.runsAsRoot {
                Text(.ConfirmationCard.confirmationCardToolRoot)
                    .foregroundStyle(Color(colors.warning))
            }
            ProblemDetail(text: tool.name + "\n\n" + tool.arguments)
                .id(view.card.id)
        }
        .wontonText(WontonTheme.typography.supporting)
    }

    @ViewBuilder private var state: some View {
        switch view.card.state {
        case .pending:
            EmptyView()
        case .running:
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text(.ConfirmationCard.confirmationCardRunning).wontonText(WontonTheme.typography.supporting)
            }
        case let .succeeded(message):
            note(message, icon: .checkCircleFill, color: colors.success)
        case let .failed(message):
            note(String(localized: .ConfirmationCard.confirmationCardFailedResult(message)), icon: .error,
                color: colors.danger)
        case let .uncertain(message):
            note(String(localized: .ConfirmationCard.confirmationCardUncertainResult(message)), icon: .help,
                color: colors.warning)
        case .rejected:
            Text(.ConfirmationCard.confirmationCardRejected)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case .expired:
            Text(.ConfirmationCard.confirmationCardExpired)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        }
    }

    @ViewBuilder private func window(_ window: WindowView) -> some View {
        switch window {
        case let .ended(note):
            Text(note)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case let .active(expiresAtMs, stopping, problem):
            let until = Date(timeIntervalSince1970: Double(expiresAtMs) / 1000)
                .formatted(date: .omitted, time: .shortened)
            HStack(spacing: WontonTheme.spacing.s8) {
                Text(.ConfirmationCard.confirmationCardWindowUntil(until)).wontonText(WontonTheme.typography.supporting)
                Button {
                    stopWindow()
                } label: {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if stopping { ProgressView() }
                        Text(.ConfirmationCard.confirmationCardStopWindow)
                    }
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .disabled(stopping)
            }
            if let problem {
                note(problem.message, icon: .warning, color: colors.danger)
            }
        }
    }

    /// 已装好且列表里查得到的插件才给"打开"；查不到（已卸载或列表没刷新）时如实不显示。
    private var installed: InstalledPlugin? {
        guard case .succeeded = view.card.state, let id = view.card.pluginId,
              let installed = plugins.viewModel.installed else { return nil }
        let plugin = installed.plugins.first { $0.id == id }
        return (plugin?.tabChoices.isEmpty == false) ? plugin : nil
    }

    /// 任一页已加进“我的应用”就不再给加的按钮。
    private var added: Bool {
        let mine = plugins.viewModel.sidebar.apps.compactMap(\.tab)
        return installed?.tabChoices.contains { mine.contains($0.tab) } == true
    }

    private func add(_ tab: TabRef) {
        pages.send(.addMyApp(app: .page(tab: tab), title: nil, index: nil))
    }

    private func note(_ text: String, icon: WontonIcon, color: Rgb) -> some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s4) {
            icon.view(size: WontonTheme.size.iconTiny)
                .padding(.top, 3)
            Text(text).wontonText(WontonTheme.typography.supporting)
        }
        .foregroundStyle(Color(color))
    }

    private var buttons: some View {
        ForEach(view.card.choices, id: \.self) { choice in
            Button {
                submit(choice)
            } label: {
                HStack(spacing: WontonTheme.spacing.s8) {
                    if view.sending == choice.answer, view.sendingChoiceId == choice.choiceId { ProgressView() }
                    Text(choice.label)
                }
            }
            .buttonStyle(WontonButtonStyle(kind: choice.answer == .approve ? .primary : .secondary))
            .disabled(view.sending != nil || (choice.answer == .approve
                && (missingRequiredSecret(view.card.secretInputs, secrets) || settingsBlocked)))
        }
    }

    private var settingsBlocked: Bool {
        (view.card.settings?.fields ?? []).contains {
            unknownField($0) || $0.problem != nil || missing($0, drafts: drafts, localErrors: localErrors)
        }
    }

    private func initializeSettings() {
        for field in view.card.settings?.fields ?? [] {
            let current = storedValue(of: field)
            switch field.kind {
            case .toggle: drafts[field.name] = .toggle(flag(of: current))
            case .integer, .decimal, .text:
                let text: String
                switch current {
                case let .integer(number): text = String(number)
                case let .decimal(value), let .text(value): text = value
                default: text = ""
                }
                drafts[field.name] = .text(text)
            case .choice(_, true):
                drafts[field.name] = .multi(PluginSettingFieldView.chosenIndices(of: current, in: field))
            case .choice, .other: break
            }
        }
    }

    private func submit(_ choice: CardChoice) {
        var values: [String: SettingValue] = [:]
        var errors: [String: String] = [:]
        for field in view.card.settings?.fields ?? [] {
            values[field.name] = storedValue(of: field)
            guard touched.contains(field.name), let draft = drafts[field.name] else { continue }
            switch submitted(field: field, draft: draft, errors: &errors) {
            case let .set(value): values[field.name] = value
            case .clear: values[field.name] = field.default
            case .skip: break
            case .invalid: break
            }
        }
        localErrors = errors
        guard choice.answer != .approve || errors.isEmpty else { return }
        answer(choice, secretValues(view.card.secretInputs, secrets), values)
    }
}

extension ConfirmationCardView {
    /// 管理员卡片已被别的管理员答过时，显示是谁处理的。
    @ViewBuilder var answeredByLine: some View {
        if view.card.audience == .admins, let answeredBy = view.card.answeredBy {
            Text(.ConfirmationCard.confirmationCardAnsweredBy(answeredBy))
                .wontonText(WontonTheme.typography.supporting)
        }
    }
}
