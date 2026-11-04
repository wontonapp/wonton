import SwiftUI
import WontonClient

/// “应用商店”：已审核的插件和社区插件分开列，可以按分类筛、按关键词找；上面是安装、升级、卸载的进展。
struct StoreView: View {
    let model: PluginsModel
    let open: (PluginEntry) -> Void
    @State private var query = ""
    @State private var category: PluginCategory?
    @State private var confirmation: PluginConfirmation?
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
                            ProblemRow(message: problem.message, nextStep: problem.reason.nextStep)
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
        .wontonPushedScreen("应用商店")
        .fullScreenCover(item: $confirmation) { InstallConfirmView(model: model, confirmation: $0) }
        .task { model.refresh(.refreshStore(force: false)) }
    }

    /// 还没取到商店：取不到的原因和下一步，或者正在取。
    @ViewBuilder private var placeholder: some View {
        if let problem = model.viewModel.problem {
            ProblemView(message: problem.message, nextStep: problem.reason.nextStep) {
                model.send(.refreshStore(force: true))
            }
        } else if let rejection = model.refreshRejection {
            ProblemView(message: rejection, nextStep: nil) { model.send(.refreshStore(force: true)) }
        } else {
            ProgressView()
        }
    }

    @ViewBuilder private func listing(_ store: StoreListing) -> some View {
        let present = categories(in: store)
        if present.count > 1 {
            CategoryChips(categories: present, selected: $category)
        }
        section("已审核", store.official)
        section("社区", store.community)
    }

    @ViewBuilder private func section(_ title: String, _ content: StoreSection) -> some View {
        let items = content.plugins.filter { category == nil || $0.category == category }
        if !items.isEmpty || content.problem != nil {
            Text(title)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.top, WontonTheme.spacing.s16)
                .padding(.bottom, WontonTheme.spacing.s4)
                .accessibilityAddTraits(.isHeader)
            ForEach(items, id: \.source) { row($0) }
            if let problem = content.problem {
                WontonNotice(icon: .warning, title: problem.reason.text)
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.vertical, WontonTheme.spacing.s8)
            }
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
        if model.viewModel.tasks.contains(where: { $0.id == item.id && $0.state == .running }) {
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

    /// 列表里出现过的分类，按第一次出现的顺序。
    private func categories(in store: StoreListing) -> [PluginCategory] {
        var seen: [PluginCategory] = []
        for item in store.official.plugins + store.community.plugins {
            if let category = item.category, !seen.contains(category) { seen.append(category) }
        }
        return seen
    }
}

private struct CategoryChips: View {
    let categories: [PluginCategory]
    @Binding var selected: PluginCategory?

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonChip(title: "全部", isSelected: selected == nil) { selected = nil }
                ForEach(categories, id: \.self) { category in
                    WontonChip(title: category.name, isSelected: selected == category) { selected = category }
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
                        icon: item.icon, category: item.category, size: WontonTheme.size.storeIcon, model: model
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
        if showsTrust || item.highPermissions || item.stars != nil {
            HStack(spacing: WontonTheme.spacing.s8) {
                if showsTrust { Tag(text: item.trust.label, tint: item.trust.tint(colors)) }
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
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: task.icon, category: task.category, size: WontonTheme.size.taskIcon, model: model)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(task.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                state.wontonText(WontonTheme.typography.supporting)
            }
            Spacer(minLength: 0)
            if task.state == .running {
                ProgressView()
            } else {
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
        case let .failed(reason, message):
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text("\(task.operation.failedText)：\(message)").foregroundStyle(Color(colors.danger))
                Text(reason.nextStep).foregroundStyle(Color(colors.textSecondary))
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
}
