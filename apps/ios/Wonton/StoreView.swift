import SwiftUI
import WontonClient

/// “应用商店”：已审核的插件和社区插件分开列，可以按分类筛、按关键词找；上面是安装、升级、卸载的进展。
struct StoreView: View {
    let model: PluginsModel
    let open: (PluginEntry) -> Void
    @State private var query = ""
    @State private var filter: String?
    @State private var confirmation: PluginConfirmation?
    @State private var showsUpdateAll = false
    @State private var isAddingSource = false
    /// “添加来源”那一页选好的文件：那一页关上以后才能再盖确认页。
    @State private var chosen: PluginConfirmation?
    @Environment(\.wontonColors) private var colors
    @Environment(\.goToInstalledApps) private var goToInstalledApps

    var body: some View {
        VStack(spacing: 0) {
            WontonSearchField(text: $query, prompt: String(localized: .storeSearchPrompt))
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.bottom, WontonTheme.spacing.s8)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.viewModel.tasks, id: \.id) { task in
                        TaskRow(model: model, task: task, review: {
                            confirmation = model.reconfirmation(for: task)
                        }, checkInstalled: { goToInstalledApps() })
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .padding(.vertical, WontonTheme.spacing.s8)
                    }
                    if let store = model.viewModel.store {
                        if let problem = model.viewModel.problem {
                            ProblemRow(problem: problem.problem) { model.send(.refreshStore(force: true)) }
                                .padding(WontonTheme.spacing.pageInset)
                        }
                        if query.isEmpty {
                            listing(store)
                        } else {
                            results
                        }
                    }
                }
            }
            .scrollDismissesKeyboard(.immediately)
            .overlay {
                if model.viewModel.store == nil { placeholder }
            }
            .refreshable { model.send(.refreshStore(force: true)) }
        }
        .wontonPushedScreen(String(localized: .storeTitle), showsStatusLine: true) {
            HStack(spacing: 0) {
                if model.viewModel.updatable.count > 0 {
                    UpdateAllButton(count: model.viewModel.updatable.count) { showsUpdateAll = true }
                }
                WontonIconButton(icon: .add, label: String(localized: .storeAddSource)) { isAddingSource = true }
            }
        }
        .fullScreenCover(item: $confirmation) { InstallConfirmView(model: model, confirmation: $0) }
        // 关掉确认页没装也没升：告诉核心放下一条（“全部更新”排队时）。
        .onChange(of: confirmation) { old, new in
            if old != nil, new == nil { model.send(.dismissStoreConfirmation) }
        }
        // 核心判断要主人拿主意的一键操作：照着打开确认页，预览已经取好。
        .onChange(of: model.viewModel.confirmation) { _, confirmation in
            self.confirmation = confirmation.map(PluginConfirmation.store)
        }
        .confirmationDialog(
            String(localized: .storeUpdateAllTitle(count: model.viewModel.updatable.count)),
            isPresented: $showsUpdateAll,
            titleVisibility: .visible
        ) {
            Button(String(localized: .storeUpdateAll)) { model.send(.upgradeAll) }
            Button(String(localized: .commonCancel), role: .cancel) {}
        }
        .fullScreenCover(isPresented: $isAddingSource, onDismiss: {
            confirmation = chosen
            chosen = nil
        }, content: {
            AddSourceView(model: model) { chosen = $0 }
        })
        .task { model.refresh(.refreshStore(force: false)) }
    }

    /// 还没取到商店：取不到的原因和下一步，或者正在取。
    @ViewBuilder private var placeholder: some View {
        if let problem = model.viewModel.problem {
            ProblemView(problem: problem.problem) { model.send(.refreshStore(force: true)) }
        } else if let rejection = model.refreshRejection {
            ProblemView(problem: rejection.problem) { model.send(.refreshStore(force: true)) }
        } else {
            ListSkeleton(iconSize: WontonTheme.size.storeIcon)
        }
    }

    @ViewBuilder private func listing(_ store: StoreListing) -> some View {
        let present = tags(in: store)
        if present.count > 1 {
            TagChips(tags: present, selected: $filter)
        }
        section(String(localized: .storeSectionReviewed), store.official)
        added(store.added)
        section(String(localized: .storeSectionCommunity), store.community)
    }

    /// 主人添加的来源合成一段，取不到的来源写明是哪一个。
    @ViewBuilder private func added(_ sources: [AddedSource]) -> some View {
        if !sources.isEmpty {
            sectionTitle(String(localized: .pluginSourcesMine))
            ForEach(sources, id: \.address) { source in
                ForEach(source.section.plugins.filter(matchesTag), id: \.source) { row($0) }
                unreadable(source.section.unreadable)
                if let problem = source.section.problem {
                    ProblemRow(
                        problem: problem.problem.titled(from: source.provider),
                        retry: problem.canRetry ? { model.send(.refreshStore(force: true)) } : nil
                    )
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.vertical, WontonTheme.spacing.s8)
                } else if source.section.fetchedAtMs == nil {
                    // 核心重启后还没读过：不显示成空。
                    Text(.storeReading(source.provider))
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .padding(.vertical, WontonTheme.spacing.s8)
                }
            }
        }
    }

    private func matchesTag(_ item: StoreItem) -> Bool {
        filter.map(item.tags.contains) ?? true
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title)
            .wontonText(WontonTheme.typography.sectionTitle)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.top, WontonTheme.spacing.s16)
            .padding(.bottom, WontonTheme.spacing.s4)
            .accessibilityAddTraits(.isHeader)
    }

    @ViewBuilder private func section(_ title: String, _ content: StoreSection) -> some View {
        let items = content.plugins.filter(matchesTag)
        if !items.isEmpty || content.problem != nil || !content.unreadable.isEmpty {
            sectionTitle(title)
            ForEach(items, id: \.source) { row($0) }
            if let problem = content.problem {
                ProblemRow(
                    problem: problem.problem,
                    retry: problem.canRetry ? { model.send(.refreshStore(force: true)) } : nil
                )
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s8)
            }
            unreadable(content.unreadable)
        }
    }

    private func unreadable(_ plugins: [StoreUnreadable]) -> some View {
        ForEach(plugins, id: \.source) { plugin in
            WontonNotice(icon: .warning, title: String(localized: .storeUnreadable(plugin.source, plugin.reason)))
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s8)
        }
    }

    @ViewBuilder private var results: some View {
        let items = model.core.searchStore(query: query)
        if items.isEmpty {
            Text(.storeNoResults)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity)
                .padding(WontonTheme.spacing.s32)
        } else {
            ForEach(items, id: \.source) { row($0) }
        }
    }

    private func row(_ item: StoreItem) -> some View {
        StoreItemRow(model: model, item: item, button: button(for: item), failure: failure(for: item))
    }

    /// 一键操作在取预览或开始之前出的错：行下面一行红字配“重试”，不弹窗（ADR-0017）。
    private func failure(for item: StoreItem) -> PluginTaskState? {
        guard !model.isWorking(on: item.id), !model.viewModel.pending.contains(item.id),
              case let .failed(reason, message) = model.viewModel.tasks.first(where: { $0.id == item.id })?.state
        else { return nil }
        return .failed(reason: reason, message: message)
    }

    /// 行尾的按钮：没装的“获取”，装着旧版本的“更新”，装着的“打开”；取预览和装、升级、卸载时转圈。点了就开始，不跳页。
    private func button(for item: StoreItem) -> StoreButton {
        if model.isWorking(on: item.id) || model.viewModel.pending.contains(item.id) {
            return .busy
        }
        guard let version = item.installedVersion else {
            return .get(enabled: item.blocked == nil) { model.send(.installNow(source: item.source)) }
        }
        if version != item.version {
            return .get(title: String(localized: .storeUpdate)) { model.send(.upgradeNow(pluginId: item.id)) }
        }
        let installed = model.viewModel.installed?.plugins.first { $0.id == item.id }
        guard let entry = installed?.entries.first else { return .hidden }
        return .get(title: String(localized: .storeOpen)) { open(entry) }
    }

    /// 列表里出现过的标签，按第一次出现的顺序。
    private func tags(in store: StoreListing) -> [String] {
        var seen: [String] = []
        for item in store.allPlugins {
            for tag in item.tags where !seen.contains(tag) { seen.append(tag) }
        }
        return seen
    }
}

private extension Problem {
    /// 添加了好几个来源时，要写明是哪一个出了问题。
    func titled(from provider: String) -> Problem {
        Problem(title: String(localized: .storeProblemFromSource(provider, title)), nextStep: nextStep, detail: detail)
    }
}

private struct TagChips: View {
    let tags: [String]
    @Binding var selected: String?

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonChip(title: String(localized: .storeTagAll), isSelected: selected == nil) { selected = nil }
                ForEach(tags, id: \.self) { tag in
                    WontonChip(title: tag, isSelected: selected == tag) { selected = tag }
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }
}

enum StoreButton {
    case get(title: String = String(localized: .storeGet), enabled: Bool = true, perform: () -> Void)
    case busy
    case hidden
}

private struct StoreItemRow: View {
    let model: PluginsModel
    let item: StoreItem
    let button: StoreButton
    /// 一键操作取预览时失败的原因；行下面一行红字配“重试”。
    var failure: PluginTaskState?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s12) {
            NavigationLink(value: PluginRoute.store(source: item.source)) {
                HStack(spacing: WontonTheme.spacing.s12) {
                    PluginIconView(
                        icon: item.icon, size: WontonTheme.size.storeIcon, model: model
                    )
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(item.title)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                            .lineLimit(1)
                        Text(item.summary)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                            .lineLimit(2)
                        tags
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            switch button {
            case let .get(title, enabled, perform):
                Button(title, action: perform)
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(!enabled)
            case .busy:
                ProgressView()
            case .hidden:
                EmptyView()
            }
            }
            if let failure {
                HStack(alignment: .firstTextBaseline, spacing: WontonTheme.spacing.s8) {
                    failureRow(failure)
                    Spacer(minLength: 0)
                    Button(String(localized: .commonRetry)) { retry() }
                        .buttonStyle(WontonSmallButtonStyle())
                }
                .padding(.leading, WontonTheme.size.storeIcon + WontonTheme.spacing.s12)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
    }

    /// 再点一次“获取”或“更新”。
    private func retry() {
        if item.installedVersion == nil {
            model.send(.installNow(source: item.source))
        } else {
            model.send(.upgradeNow(pluginId: item.id))
        }
    }

    @ViewBuilder private func failureRow(_ state: PluginTaskState) -> some View {
        if case let .failed(reason, message) = state {
            let problem = reason.problem(detail: message)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(problem.title)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
                ProblemDetail(text: message)
            }
        }
    }

    /// 已审核的插件在自己的分组里，不再标来源。
    @ViewBuilder private var tags: some View {
        let showsTrust = item.trust != .official
        let highTier = item.securityTier.map { $0 != .minimal && $0 != .standard } ?? false
        if showsTrust || item.provider != nil || highTier || item.stars != nil {
            HStack(spacing: WontonTheme.spacing.s8) {
                if showsTrust { Tag(text: item.trust.label, tint: item.trust.tint(colors)) }
                if let provider = item.viaIndex.map({ String(localized: .storeViaIndex($0)) }) ?? item.provider {
                    Text(provider)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
                if highTier { Tag(text: String(localized: .storeHighPermissions), tint: colors.danger) }
                if let stars = item.stars {
                    HStack(spacing: WontonTheme.spacing.s4) {
                        WontonIcon.starFill.view(size: WontonTheme.size.iconTiny)
                            .accessibilityLabel(String(localized: .pluginInfoStars))
                        Text(stars.formatted(.number.notation(.compactName)))
                            .wontonText(WontonTheme.typography.caption)
                    }
                    .foregroundStyle(Color(colors.textSecondary))
                }
            }
        }
    }
}

struct TaskRow: View {
    let model: PluginsModel
    let task: PluginTask
    let review: () -> Void
    /// The result is uncertain: go look in the installed apps instead of installing again.
    var checkInstalled: () -> Void = {}
    var showsDismiss = true
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: task.icon, size: WontonTheme.size.taskIcon, model: model)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(task.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                state.wontonText(WontonTheme.typography.supporting)
            }
            Spacer(minLength: 0)
            if task.state == .running {
                ProgressView()
            } else if showsDismiss {
                WontonIconButton(icon: .close, label: String(localized: .commonClose)) {
                    model.send(.dismissPluginTask(id: task.id))
                }
            }
        }
    }

    @ViewBuilder private var state: some View {
        switch task.state {
        case .running:
            Text(
                task.waitingForConnection
                    ? String(localized: .pluginTaskWaitingForConnection)
                    : task.operation.runningText
            )
            .foregroundStyle(Color(colors.textSecondary))
        case .succeeded:
            HStack(spacing: WontonTheme.spacing.s4) {
                WontonIcon.checkCircleFill.view(size: WontonTheme.size.iconTiny)
                Text(task.operation.doneText)
            }
            .foregroundStyle(Color(colors.success))
            addedTools
        case let .failed(reason, message):
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                let problem = reason.problem(detail: message)
                let failed = switch task.operation {
                case .install: String(localized: .pluginTaskInstallFailed(problem.title))
                case .upgrade: String(localized: .pluginTaskUpgradeFailed(problem.title))
                case .uninstall: String(localized: .pluginTaskUninstallFailed(problem.title))
                }
                Text(failed).foregroundStyle(Color(colors.danger))
                if let nextStep = problem.nextStep {
                    Text(nextStep).foregroundStyle(Color(colors.textSecondary))
                }
                ProblemDetail(text: message)
                if reason == .changed {
                    Button(String(localized: .pluginTaskReview), action: review).buttonStyle(WontonSmallButtonStyle())
                }
            }
        case let .uncertain(message, nextStep):
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                HStack(alignment: .top, spacing: WontonTheme.spacing.s4) {
                    WontonIcon.help.view(size: WontonTheme.size.iconTiny).padding(.top, 3)
                    Text(.pluginTaskUncertain(message))
                }
                .foregroundStyle(Color(colors.warning))
                if nextStep == .checkInstalled {
                    Button(String(localized: .pluginTaskCheckInstalled), action: checkInstalled)
                        .buttonStyle(WontonSmallButtonStyle())
                }
            }
        }
    }

    @ViewBuilder private var addedTools: some View {
        switch task.addedTools {
        case let .compared(names) where !names.isEmpty:
            Text(.pluginTaskAddedTools(names.formatted(.list(type: .and))))
                .foregroundStyle(Color(colors.textSecondary))
        case let .notCompared(reason):
            Text(.pluginTaskAddedToolsUnknown).foregroundStyle(Color(colors.warning))
            ProblemDetail(text: reason)
        default:
            EmptyView()
        }
    }
}

extension PluginTask {
    /// 详情页重取后已经看得出升级成功，但多了哪些工具只有这一行会说。
    var hasAddedToolsNews: Bool {
        switch addedTools {
        case let .compared(names): !names.isEmpty
        case .notCompared: true
        case nil: false
        }
    }
}

/// 商店页右上角的更新图标（向下的箭头），带红色数字角标；点了弹一次确认。
private struct UpdateAllButton: View {
    let count: Int
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        WontonIconButton(icon: .download, label: String(localized: .storeUpdateAll)) { action() }
            .overlay(alignment: .topTrailing) {
                Text("\(count)")
                    .font(.system(size: 11, weight: .semibold).monospacedDigit())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 5)
                    .frame(minWidth: 18, minHeight: 18)
                    .background(Color(colors.danger), in: Capsule())
                    .offset(x: 4, y: -4)
                    .accessibilityLabel(String(localized: .launcherUpdatesAvailable(count: count)))
            }
    }
}

extension EnvironmentValues {
    /// 关掉叠着的页面、回到已安装的应用格子（结果不确定时用）。
    @Entry var goToInstalledApps: () -> Void = {}
}
