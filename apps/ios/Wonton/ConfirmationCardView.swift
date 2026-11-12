import SwiftUI
import WontonClient

/// 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。
struct ConfirmationCardView: View {
    let view: ConfirmationView
    let answer: (CardChoice, [String: String]) -> Void
    let stopWindow: () -> Void
    @Environment(TabsModel.self) private var tabs
    @Environment(PluginsModel.self) private var plugins
    @State private var showsPages = false
    @State private var pinRequest: TabRef?
    @State private var secrets: [String: String] = [:]
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
            if !view.card.secretInputs.isEmpty {
                SecretInputsSection(
                    inputs: view.card.secretInputs, values: secrets,
                    enabled: view.card.state == .pending
                ) { name, value in
                    secrets[name] = value
                }
            }
            state
            if let window = view.window {
                self.window(window)
            }
            if let problem = view.problem {
                note(problem.message, icon: .warning, color: colors.danger)
            }
            if let installed {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Button(TabText.open) { tabs.select(installed.tabChoices[0].tab) }
                        .buttonStyle(WontonSmallButtonStyle())
                    if !pinned {
                        Button(TabText.pinToBar) {
                            if installed.tabChoices.count > 1 {
                                showsPages = true
                            } else {
                                pinRequest = installed.tabChoices[0].tab
                            }
                        }
                        .buttonStyle(WontonSmallButtonStyle())
                    }
                }
                .wontonSheet(isPresented: $showsPages, title: installed.title) { _ in
                    PluginTabPins(plugin: installed).padding(WontonTheme.spacing.pageInset)
                }
                .tabPinRequests($pinRequest)
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
    }

    @ViewBuilder private var state: some View {
        switch view.card.state {
        case .pending:
            EmptyView()
        case .running:
            HStack(spacing: WontonTheme.spacing.s8) {
                ProgressView()
                Text(.confirmationCardRunning).wontonText(WontonTheme.typography.supporting)
            }
        case let .succeeded(message):
            note(message, icon: .checkCircleFill, color: colors.success)
        case let .failed(message):
            note(String(localized: .confirmationCardFailedResult(message)), icon: .error, color: colors.danger)
        case let .uncertain(message):
            note(String(localized: .confirmationCardUncertainResult(message)), icon: .help, color: colors.warning)
        case .rejected:
            Text(.confirmationCardRejected)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
        case .expired:
            Text(.confirmationCardExpired)
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
                Text(.confirmationCardWindowUntil(until)).wontonText(WontonTheme.typography.supporting)
                Button {
                    stopWindow()
                } label: {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if stopping { ProgressView() }
                        Text(.confirmationCardStopWindow)
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

    /// 已装好且列表里查得到的插件才给"打开/放到底栏"；查不到（已卸载或列表没刷新）时如实不显示。
    private var installed: InstalledPlugin? {
        guard case .succeeded = view.card.state, let id = view.card.pluginId,
              let installed = plugins.viewModel.installed else { return nil }
        let plugin = installed.plugins.first { $0.id == id }
        return (plugin?.tabChoices.isEmpty == false) ? plugin : nil
    }

    private var pinned: Bool {
        installed?.tabChoices.contains { tabs.isPinned($0.tab) } == true
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
                answer(choice, secretValues(view.card.secretInputs, secrets))
            } label: {
                HStack(spacing: WontonTheme.spacing.s8) {
                    if view.sending == choice.answer, view.sendingChoiceId == choice.choiceId { ProgressView() }
                    Text(choice.label)
                }
            }
            .buttonStyle(WontonButtonStyle(kind: choice.answer == .approve ? .primary : .secondary))
            .disabled(view.sending != nil || (choice.answer == .approve
                && missingRequiredSecret(view.card.secretInputs, secrets)))
        }
    }
}
