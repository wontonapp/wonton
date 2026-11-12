import SwiftUI
import WontonClient

/// “应用商店”：已审核的插件和社区插件分开列，可以按分类筛、按关键词找；上面是安装、升级、卸载的进展。
struct StoreView: View {
    let model: PluginsModel
    let open: (PluginEntry) -> Void
    @State private var query = ""
    @State private var filter: String?
    @State private var confirmation: PluginConfirmation?
    @State private var isAddingSource = false
    /// “添加来源”那一页选好的文件：那一页关上以后才能再盖确认页。
    @State private var chosen: PluginConfirmation?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: 0) {
            WontonSearchField(text: $query, prompt: "搜索")
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.bottom, WontonTheme.spacing.s8)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.viewModel.tasks, id: \.id) { task in
                        TaskRow(model: model, task: task) {
                            confirmation = model.reconfirmation(for: task)
                        }
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
        .wontonPushedScreen("应用商店", showsStatusLine: true) {
            WontonIconButton(icon: .add, label: "添加来源") { isAddingSource = true }
        }
        .fullScreenCover(item: $confirmation) { InstallConfirmView(model: model, confirmation: $0) }
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
        section("已审核", store.official)
        added(store.added)
        section("社区", store.community)
    }

    /// 主人添加的来源合成一段，取不到的来源写明是哪一个。
    @ViewBuilder private func added(_ sources: [AddedSource]) -> some View {
        if !sources.isEmpty {
            sectionTitle("我添加的")
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
                    Text("\(source.provider)：正在读")
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
            WontonNotice(icon: .warning, title: "\(plugin.source) 读不懂：\(plugin.reason)")
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s8)
        }
    }

    @ViewBuilder private var results: some View {
        let items = model.core.searchStore(query: query)
        if items.isEmpty {
            Text("没有找到")
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity)
                .padding(WontonTheme.spacing.s32)
        } else {
            ForEach(items, id: \.source) { row($0) }
        }
    }

    private func row(_ item: StoreItem) -> some View {
        StoreItemRow(model: model, item: item, button: button(for: item))
    }

    /// 行尾的按钮：没装的“获取”，装着旧版本的“更新”，装着的“打开”；正在装、升级、卸载时转圈。
    private func button(for item: StoreItem) -> StoreButton {
        if model.isWorking(on: item.id) {
            return .busy
        }
        guard let version = item.installedVersion else {
            return .get(enabled: item.blocked == nil) { confirmation = .install(source: item.source) }
        }
        if version != item.version {
            return .get(title: "更新") { confirmation = .upgrade(id: item.id) }
        }
        let installed = model.viewModel.installed?.plugins.first { $0.id == item.id }
        guard let entry = installed?.entries.first else { return .hidden }
        return .get(title: "打开") { open(entry) }
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
        Problem(title: "\(provider)：\(title)", nextStep: nextStep, detail: detail)
    }
}

private struct TagChips: View {
    let tags: [String]
    @Binding var selected: String?

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonChip(title: "全部", isSelected: selected == nil) { selected = nil }
                ForEach(tags, id: \.self) { tag in
                    WontonChip(title: tag, isSelected: selected == tag) { selected = tag }
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }
}

enum StoreButton {
    case get(title: String = "获取", enabled: Bool = true, perform: () -> Void)
    case busy
    case hidden
}

private struct StoreItemRow: View {
    let model: PluginsModel
    let item: StoreItem
    let button: StoreButton
    @Environment(\.wontonColors) private var colors

    var body: some View {
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
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
    }

    /// 已审核的插件在自己的分组里，不再标来源。
    @ViewBuilder private var tags: some View {
        let showsTrust = item.trust != .official
        if showsTrust || item.provider != nil || item.highPermissions || item.stars != nil {
            HStack(spacing: WontonTheme.spacing.s8) {
                if showsTrust { Tag(text: item.trust.label, tint: item.trust.tint(colors)) }
                if let provider = item.viaIndex.map({ "经 \($0)" }) ?? item.provider {
                    Text(provider)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
                if item.highPermissions { Tag(text: "高权限", tint: colors.danger) }
                if let stars = item.stars {
                    HStack(spacing: WontonTheme.spacing.s4) {
                        WontonIcon.starFill.view(size: WontonTheme.size.iconTiny)
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
                WontonIconButton(icon: .close, label: "关闭") { model.send(.dismissPluginTask(id: task.id)) }
            }
        }
    }

    @ViewBuilder private var state: some View {
        switch task.state {
        case .running:
            Text(task.operation.runningText).foregroundStyle(Color(colors.textSecondary))
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
                Text("\(task.operation.failedText)：\(problem.title)").foregroundStyle(Color(colors.danger))
                if let nextStep = problem.nextStep {
                    Text(nextStep).foregroundStyle(Color(colors.textSecondary))
                }
                ProblemDetail(text: message)
                if reason == .changed {
                    Button("重新查看", action: review).buttonStyle(WontonSmallButtonStyle())
                }
            }
        case let .uncertain(message):
            HStack(alignment: .top, spacing: WontonTheme.spacing.s4) {
                WontonIcon.help.view(size: WontonTheme.size.iconTiny).padding(.top, 3)
                Text("结果不确定：\(message)")
            }
            .foregroundStyle(Color(colors.warning))
        }
    }

    @ViewBuilder private var addedTools: some View {
        switch task.addedTools {
        case let .compared(names) where !names.isEmpty:
            Text("新增的工具：\(names.joined(separator: "、"))")
                .foregroundStyle(Color(colors.textSecondary))
        case let .notCompared(reason):
            Text("不知道这次多了哪些工具").foregroundStyle(Color(colors.warning))
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
