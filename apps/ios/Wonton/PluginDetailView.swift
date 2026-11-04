import SwiftUI
import WontonClient

/// 插件的详情页：商店里的看来源最新的版本，主屏上的看装着的版本。
struct PluginDetailView: View {
    let model: PluginsModel
    let route: PluginRoute
    let open: (PluginEntry) -> Void
    @State private var loaded: Result<PluginDetail, Error>?
    @State private var confirmation: PluginConfirmation?
    @State private var uninstalling: UninstallTarget?
    @State private var pluginId: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        content
            .navigationBarTitleDisplayMode(.inline)
            .task(id: route) { await load() }
            .sheet(item: $confirmation) { InstallConfirmView(model: model, confirmation: $0) }
            .uninstallDialog($uninstalling, model: model)
            .onChange(of: isWorking) { wasWorking, working in
                if wasWorking, !working { taskEnded() }
            }
    }

    private var task: PluginTask? {
        model.viewModel.tasks.first { $0.id == pluginId }
    }

    private var isWorking: Bool {
        task?.state == .running
    }

    /// 详情只在打开时取一次，任务做完要重取，按钮才对。主屏上打开的插件卸载以后，这一页没有东西可看了。
    private func taskEnded() {
        if case .installed = route, task?.operation == .uninstall, task?.state == .succeeded {
            dismiss()
        } else {
            Task { await load() }
        }
    }

    @ViewBuilder private var content: some View {
        switch loaded {
        case nil:
            ProgressView()
        case let .failure(error):
            ProblemView(message: String(describing: error), nextStep: nil) {
                Task { await load() }
            }
        case let .success(detail):
            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    DetailHeader(model: model, detail: detail, action: action(for: detail))
                    if let task, task.state != .succeeded {
                        TaskRow(model: model, task: task) {
                            confirmation = model.reconfirmation(for: task)
                        }
                    }
                    if let installed = detail.installed, !installed.notices.isEmpty {
                        VStack(alignment: .leading, spacing: 8) { NoticeLines(lines: installed.notices) }
                    }
                    if !detail.screenshots.isEmpty {
                        Screenshots(model: model, images: detail.screenshots)
                    }
                    if let about = detail.about { Text(about) }
                    if let installed = detail.installed, let offer = detail.lanWebui {
                        InstalledLanWebui(model: model, id: installed.id, offer: offer) { loaded = .success($0) }
                    }
                    DetailSections(detail: detail)
                    if let installed = detail.installed, installed.removable, !isWorking {
                        Button("卸载", role: .destructive) {
                            uninstalling = UninstallTarget(id: installed.id, title: installed.title)
                        }
                    }
                }
                .padding()
            }
        }
    }

    private func load() async {
        loaded = nil
        do {
            switch route {
            case let .store(source):
                loaded = .success(try await model.core.storePlugin(source: source))
                pluginId = try loaded?.get().id
            case let .installed(id):
                pluginId = id
                loaded = .success(try await model.core.installedPlugin(id: id))
            }
        } catch {
            loaded = .failure(error)
        }
    }

    /// 标题旁的按钮：没装的“获取”，能升级的“更新”，装着的“打开”。
    private func action(for detail: PluginDetail) -> DetailAction? {
        if let installed = detail.installed {
            if installed.upgrade != nil {
                return DetailAction(title: "更新", enabled: !installed.isBusy) {
                    confirmation = .upgrade(id: installed.id)
                }
            }
            guard let entry = installed.entries.first else { return nil }
            return DetailAction(title: "打开", enabled: !installed.isBusy) { open(entry) }
        }
        guard let source = detail.source else { return nil }
        return DetailAction(title: "获取", enabled: detail.blocked == nil && !isWorking) {
            confirmation = .install(source: source)
        }
    }
}

/// 已装插件的开关：核心重建容器期间不能再点，失败时写原因，并重取详情（关失败时插件标为异常）。
private struct InstalledLanWebui: View {
    let model: PluginsModel
    let id: String
    let offer: LanWebuiOffer
    let updated: (PluginDetail) -> Void
    @State private var target: Bool?
    @State private var problem: String?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            LanWebuiToggle(
                offer: offer,
                isOn: Binding(get: { target ?? offer.enabled }, set: { change(to: $0) })
            )
            .disabled(target != nil)
            if target != nil { ProgressView() }
            if let problem { Text(problem).foregroundStyle(Color(colors.danger)) }
        }
    }

    private func change(to enabled: Bool) {
        target = enabled
        problem = nil
        Task {
            do {
                updated(try await model.core.setLanWebui(id: id, enabled: enabled))
            } catch {
                problem = String(describing: error)
                if let detail = try? await model.core.installedPlugin(id: id) { updated(detail) }
            }
            target = nil
        }
    }
}

private struct DetailAction {
    let title: String
    let enabled: Bool
    let perform: () -> Void
}

private struct DetailHeader: View {
    let model: PluginsModel
    let detail: PluginDetail
    let action: DetailAction?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 16) {
                PluginIconView(icon: detail.icon, category: detail.category, size: 112, model: model)
                VStack(alignment: .leading, spacing: 6) {
                    Text(detail.title).font(.title2.bold())
                    Text(detail.summary).font(.subheadline).foregroundStyle(.secondary)
                    Tag(text: detail.trust.label, tint: detail.trust.tint)
                    if let action {
                        Button(action.title, action: action.perform)
                            .buttonStyle(.borderedProminent)
                            .buttonBorderShape(.capsule)
                            .disabled(!action.enabled)
                    }
                }
            }
            if let blocked = detail.blocked {
                Label(blocked, systemImage: "nosign").font(.callout).foregroundStyle(.red)
            }
            if let installed = detail.installed, case let .degraded(reason) = installed.status {
                Label(reason, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.red)
            }
        }
    }
}

private struct Screenshots: View {
    let model: PluginsModel
    let images: [PluginImage]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 12) {
                ForEach(images, id: \.path) { Screenshot(model: model, source: $0) }
            }
        }
    }
}

/// 一张截图；取不到时画占位（ADR-0017）。
private struct Screenshot: View {
    let model: PluginsModel
    let source: PluginImage
    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image).resizable().scaledToFit()
            } else {
                Color(.secondarySystemBackground)
                    .aspectRatio(9 / 19.5, contentMode: .fit)
                    .overlay { Image(systemName: "photo").foregroundStyle(.tertiary) }
            }
        }
        .frame(height: 380)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .task(id: source.path) { image = try? await model.image(source) }
    }
}

/// 更新说明、权限、要求和其他信息。
private struct DetailSections: View {
    let detail: PluginDetail

    var body: some View {
        if let notes = detail.releaseNotes {
            DetailSection(title: "新内容") {
                Text(detail.version).font(.subheadline).foregroundStyle(.secondary)
                Text(notes)
            }
        }
        if !detail.permissions.isEmpty {
            DetailSection(title: "权限") {
                ForEach(detail.permissions, id: \.permission) { PermissionRow(line: $0) }
            }
        }
        if !detail.requirements.isEmpty {
            DetailSection(title: "要求") {
                ForEach(detail.requirements, id: \.self) { Label($0, systemImage: "checklist") }
            }
        }
        DetailSection(title: "信息") {
            LabeledContent("版本", value: detail.version)
            if let category = detail.category { LabeledContent("分类", value: category.name) }
            if let author = detail.author { LabeledContent("作者", value: author.name) }
            if let license = detail.license { LabeledContent("许可证", value: license) }
            if let source = detail.source { LabeledContent("来源", value: source) }
            if let stars = detail.stars { LabeledContent("星标", value: stars.formatted()) }
            if let link = detail.homepage.flatMap(URL.init(string:)) { Link("主页", destination: link) }
            if let link = detail.repository.flatMap(URL.init(string:)) { Link("代码仓库", destination: link) }
        }
    }
}

private struct DetailSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.title3.bold())
            content
        }
    }
}

extension View {
    /// 卸载前确认：可以保留数据，也可以连数据一起删。
    func uninstallDialog(_ target: Binding<UninstallTarget?>, model: PluginsModel) -> some View {
        confirmationDialog(
            "卸载“\(target.wrappedValue?.title ?? "")”？",
            isPresented: Binding(
                get: { target.wrappedValue != nil },
                set: { isShowing in
                    if !isShowing { target.wrappedValue = nil }
                }
            ),
            titleVisibility: .visible,
            presenting: target.wrappedValue
        ) { plugin in
            Button("卸载，保留数据") { model.send(.uninstallPlugin(id: plugin.id, deleteData: false)) }
            Button("卸载并删除数据", role: .destructive) {
                model.send(.uninstallPlugin(id: plugin.id, deleteData: true))
            }
        } message: { _ in
            Text("删除数据会一并删掉它的密钥和 Bot 账号，不能恢复。")
        }
    }
}
