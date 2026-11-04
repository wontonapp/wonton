import SwiftUI
import WontonClient

/// “商店”：已审核的插件和社区插件分开列，可以按分类筛、按关键词找；上面是安装、升级、卸载的进展。
struct StoreView: View {
    let model: PluginsModel
    let open: (PluginEntry) -> Void
    @State private var path: [PluginRoute] = []
    @State private var query = ""
    @State private var category: PluginCategory?
    @State private var confirmation: PluginConfirmation?

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if !model.viewModel.tasks.isEmpty {
                    Section {
                        ForEach(model.viewModel.tasks, id: \.id) { task in
                            TaskRow(model: model, task: task) {
                                confirmation = model.reconfirmation(for: task)
                            }
                        }
                    }
                }
                if let store = model.viewModel.store {
                    if let problem = model.viewModel.problem {
                        ProblemRow(message: problem.message, nextStep: problem.reason.nextStep)
                    }
                    if query.isEmpty {
                        listing(store)
                    } else {
                        results
                    }
                }
            }
            .overlay {
                if model.viewModel.store == nil { placeholder }
            }
            .navigationTitle("商店")
            .searchable(text: $query, prompt: "搜索")
            .refreshable { model.send(.refreshStore(force: true)) }
            .navigationDestination(for: PluginRoute.self) { PluginDetailView(model: model, route: $0, open: open) }
            .sheet(item: $confirmation) { InstallConfirmView(model: model, confirmation: $0) }
            .alert("操作被拒绝", isPresented: isShowingRejection, presenting: model.rejection) { _ in
                Button("好") {}
            } message: { reason in
                Text(reason)
            }
        }
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
            Section {
                CategoryChips(categories: present, selected: $category)
            }
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)
        }
        section("已审核", store.official)
        section("社区", store.community)
    }

    @ViewBuilder private func section(_ title: String, _ content: StoreSection) -> some View {
        let items = content.plugins.filter { category == nil || $0.category == category }
        if !items.isEmpty || content.problem != nil {
            Section {
                ForEach(items, id: \.source) { row($0) }
            } header: {
                Text(title)
            } footer: {
                if let problem = content.problem { Text(problem.message) }
            }
        }
    }

    @ViewBuilder private var results: some View {
        let items = model.core.searchStore(query: query)
        if items.isEmpty {
            ContentUnavailableView.search(text: query)
        } else {
            ForEach(items, id: \.source) { row($0) }
        }
    }

    private func row(_ item: StoreItem) -> some View {
        NavigationLink(value: PluginRoute.store(source: item.source)) {
            StoreItemRow(model: model, item: item, button: button(for: item))
        }
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

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { model.rejection != nil },
            set: { isShowing in
                if !isShowing { model.rejection = nil }
            }
        )
    }
}

private struct CategoryChips: View {
    let categories: [PluginCategory]
    @Binding var selected: PluginCategory?

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("全部", isSelected: selected == nil) { selected = nil }
                ForEach(categories, id: \.self) { category in
                    chip(category.name, isSelected: selected == category) { selected = category }
                }
            }
            .padding(.horizontal)
            .padding(.vertical, 4)
        }
    }

    private func chip(_ title: String, isSelected: Bool, select: @escaping () -> Void) -> some View {
        Button(title, action: select)
            .font(.subheadline.weight(.medium))
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
            .foregroundStyle(isSelected ? Color.white : Color.primary)
            .background(isSelected ? Color.accentColor : Color(.secondarySystemFill), in: Capsule())
            .buttonStyle(.plain)
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

    var body: some View {
        HStack(spacing: 12) {
            PluginIconView(icon: item.icon, category: item.category, size: 56, model: model)
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title).font(.headline).lineLimit(1)
                Text(item.summary).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                tags
            }
            Spacer(minLength: 8)
            switch button {
            case let .get(title, enabled, perform):
                Button(title, action: perform)
                    .font(.subheadline.bold())
                    .buttonStyle(.bordered)
                    .buttonBorderShape(.capsule)
                    .disabled(!enabled)
            case .busy:
                ProgressView()
            case .hidden:
                EmptyView()
            }
        }
        .padding(.vertical, 2)
    }

    /// 已审核的插件在自己的分组里，不再标来源。
    @ViewBuilder private var tags: some View {
        let showsTrust = item.trust != .official
        if showsTrust || item.highPermissions || item.stars != nil {
            HStack(spacing: 6) {
                if showsTrust { Tag(text: item.trust.label, tint: item.trust.tint) }
                if item.highPermissions { Tag(text: "高权限", tint: .red) }
                if let stars = item.stars {
                    Label(stars.formatted(.number.notation(.compactName)), systemImage: "star.fill")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct TaskRow: View {
    let model: PluginsModel
    let task: PluginTask
    let review: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            PluginIconView(icon: task.icon, category: nil, size: 40, model: model)
            VStack(alignment: .leading, spacing: 2) {
                Text(task.title).font(.subheadline.weight(.semibold))
                state
            }
            Spacer(minLength: 8)
            if task.state == .running {
                ProgressView()
            } else {
                Button("关闭", systemImage: "xmark.circle.fill") { model.send(.dismissPluginTask(id: task.id)) }
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.secondary)
                    .buttonStyle(.borderless)
            }
        }
    }

    @ViewBuilder private var state: some View {
        switch task.state {
        case .running:
            Text(task.operation.runningText).font(.footnote).foregroundStyle(.secondary)
        case .succeeded:
            Label(task.operation.doneText, systemImage: "checkmark.circle.fill")
                .font(.footnote)
                .foregroundStyle(.green)
        case let .failed(reason, message):
            VStack(alignment: .leading, spacing: 2) {
                Text("\(task.operation.failedText)：\(message)").foregroundStyle(.red)
                Text(reason.nextStep).foregroundStyle(.secondary)
                if reason == .changed {
                    Button("重新查看", action: review).buttonStyle(.borderless)
                }
            }
        case let .uncertain(message):
            Label("结果不确定：\(message)", systemImage: "questionmark.circle").foregroundStyle(.orange)
                .font(.footnote)
        }
    }
}
