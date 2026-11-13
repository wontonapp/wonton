import SwiftUI
import WontonClient

/// 详情页上密钥一节的滚动锚点：缺必填项时“去填写”滚到这里。
private let pluginSecretsAnchor = "pluginSecrets"

/// 插件的详情页：商店里的看来源最新的版本，主屏上的看装着的版本。
struct PluginDetailView: View {
    let model: PluginsModel
    let chat: ChatModel
    let route: PluginRoute
    let open: (PluginEntry) -> Void
    @State private var loaded: Result<PluginDetail, Error>?
    @State private var confirmation: PluginConfirmation?
    @State private var uninstalling: UninstallTarget?
    @State private var pluginId: String?
    @State private var changingVersionOf: String?
    @State private var showsSettings = false
    @State private var showsContacts = false
    @State private var isOwner = false
    @State private var secretsScrollTarget: String?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.wontonColors) private var colors
    @Environment(\.goToInstalledApps) private var goToInstalledApps

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wontonPushedScreen("", showsStatusLine: true)
            .task(id: route) { await load() }
            .task { isOwner = (try? await model.core.me())?.isOwner == true }
            .fullScreenCover(item: $confirmation) { InstallConfirmView(model: model, confirmation: $0) }
            .changeVersionCover(of: $changingVersionOf, model: model, confirmation: $confirmation)
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

    /// 详情只在打开时取一次，任务做完要重取，按钮才对。要不要关这一页由核心判断：插件已经卸掉了，核心让关。
    private func taskEnded() {
        Task {
            do {
                switch try await model.core.pluginDetailAfterTask(route: route.coreRoute) {
                case .close: dismiss()
                case let .detail(detail): loaded = .success(detail)
                }
            } catch {
                loaded = .failure(error)
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch loaded {
        case nil:
            LoadingView()
        case let .failure(error):
            ProblemView(problem: error.problem) {
                Task { await load() }
            }
        case let .success(detail):
            ScrollView {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                    if detail.installed != nil {
                        BotProfileSection(chat: chat, plugins: model, pluginId: detail.id)
                    }
                    if detail.installed != nil, bot(of: detail.id) != nil {
                        WontonGroup {
                            Button {
                                showsContacts = true
                            } label: {
                                WontonRow(title: String(localized: .Contacts0.contactsTitle), showsForward: true)
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("botContactsEntry")
                        }
                    }
                    DetailHeader(model: model, detail: detail, action: action(for: detail))
                    if let task, task.state != .succeeded || task.hasAddedToolsNews {
                        TaskRow(
                            model: model,
                            task: task,
                            review: {
                                confirmation = model.reconfirmation(for: task)
                            },
                            checkInstalled: goToInstalledApps,
                        )
                    }
                    if let installed = detail.installed, !installed.notices.isEmpty {
                        NoticeLines(lines: installed.notices)
                    }
                    if let installed = detail.installed, case .needsConfiguration = installed.status, isOwner {
                        Button {
                            secretsScrollTarget = pluginSecretsAnchor
                        } label: {
                            Text(.Plugins.secretGoFill).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                    if let installed = detail.installed { PluginTabPins(plugin: installed) }
                    if detail.installed != nil, detail.hasSettings {
                        WontonGroup {
                            Button {
                                showsSettings = true
                            } label: {
                                WontonRow(title: String(localized: .Plugin1.pluginSettingsTitle), showsForward: true)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    if !detail.screenshots.isEmpty {
                        Screenshots(model: model, images: detail.screenshots)
                    }
                    if let about = detail.about {
                        Text(about)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                    if detail.installed != nil {
                        SecretSection(detail: detail, model: model, isOwner: isOwner) { loaded = .success($0) }
                            .id(pluginSecretsAnchor)
                    }
                    if let installed = detail.installed, let offer = detail.lanWebui {
                        InstalledLanWebui(model: model, id: installed.id, offer: offer) { loaded = .success($0) }
                    }
                    if let installed = detail.installed, !installed.tunnelPorts.isEmpty {
                        PortMappingSection(installed: installed)
                    }
                    if let installed = detail.installed, !detail.saveGrants.isEmpty {
                        SaveGrants(model: model, id: installed.id, folders: detail.saveGrants) { loaded = .success($0) }
                    }
                    if let shares = detail.shares {
                        PluginSharesSection(model: model, pluginId: detail.id, shares: shares) {
                            loaded = .success($0)
                        }
                    }
                    if let installed = detail.installed {
                        ToolPolicySection(model: model, id: installed.id, version: detail.version)
                    }
                    if let installed = detail.installed, installed.writtenBy != nil {
                        AskEveryUpdateSection(model: model, id: installed.id, enabled: detail.askEveryUpdate ?? false)
                    }
                    DetailSections(detail: detail, model: model) { loaded = .success($0) }
                    if let installed = detail.installed, installed.removable, !isWorking {
                        Button {
                            changingVersionOf = installed.id
                        } label: {
                            Text(.Install.installChangeVersion).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                        Button(role: .destructive) {
                            uninstalling = UninstallTarget(id: installed.id, title: installed.title)
                        } label: {
                            Text(.Plugin3.pluginUninstallAction).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .danger))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(WontonTheme.spacing.pageInset)
                .scrollTargetLayout()
            }
            .scrollPosition(id: $secretsScrollTarget, anchor: .top)
            .navigationDestination(isPresented: $showsSettings) {
                PluginSettingsScreen(model: model, pluginId: pluginId ?? "")
            }
            .navigationDestination(isPresented: $showsContacts) {
                if let bot = bot(of: detail.id) { BotContactsView(core: model.core, bot: bot) }
            }
        }
    }

    /// 有 Bot 的应用的那个 Bot（和资料一节同一个）：它的联系人页从这里进（ADR-0045）。
    private func bot(of pluginId: String) -> BotRef? {
        chat.viewModel.bots.first { $0.bot.pluginId == pluginId && $0.bot.bot == "default" }?.bot
    }

    private func load() async {
        loaded = nil
        // 主屏打开的插件先知道 ID：取详情的请求还没回来、任务就做完时，也要看得到“进行中 → 结束”这一步。
        if case let .installed(id) = route { pluginId = id }
        do {
            switch try await model.core.pluginDetail(route: route.coreRoute) {
            case .close:
                dismiss()
            case let .detail(detail):
                pluginId = detail.id
                loaded = .success(detail)
            }
        } catch {
            loaded = .failure(error)
        }
    }

    /// 标题旁的按钮：没装的“获取”，能升级的“更新”，装着的“打开”。
    private func action(for detail: PluginDetail) -> DetailAction? {
        if let installed = detail.installed {
            if installed.upgrade != nil {
                return DetailAction(title: String(localized: .Store.storeUpdate), enabled: !installed.isBusy) {
                    confirmation = .upgrade(id: installed.id)
                }
            }
            guard let entry = installed.entries.first else { return nil }
            return DetailAction(title: String(localized: .Store.storeOpen), enabled: !installed.isBusy) { open(entry) }
        }
        guard let source = detail.source else { return nil }
        return DetailAction(title: String(localized: .Store.storeGet), enabled: detail.blocked == nil && !isWorking) {
            confirmation = .install(source: source)
        }
    }
}

/// Bot 能存文件的路径（ADR-0027）。撤销只拦后面的调用，已经放进去的文件不动。
private struct SaveGrants: View {
    let model: PluginsModel
    let id: String
    let folders: [String]
    let updated: (PluginDetail) -> Void
    @State private var revoking = false
    @State private var problem: String?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Plugin1.pluginSaveGrantTitle)) {
            ForEach(folders, id: \.self) { folder in
                HStack(spacing: WontonTheme.spacing.s12) {
                    Text(folder)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Button(.Plugin3.pluginSaveGrantRevoke) { revoke(folder) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                        .fixedSize()
                        .disabled(revoking)
                }
            }
            if let problem { WontonNotice(icon: .warning, title: problem) }
        }
    }

    private func revoke(_ folder: String) {
        revoking = true
        problem = nil
        Task {
            do {
                updated(try await model.core.revokeSaveGrant(id: id, folder: folder))
            } catch {
                problem = String(describing: error)
            }
            revoking = false
        }
    }
}

private struct DetailAction {
    let title: String
    let enabled: Bool
    let perform: () -> Void
}

/// “每次都问”（ADR-0033）：打开后这个 Agent 每次更新都发确认卡片。只有 Agent 写的插件有这一节。
private struct AskEveryUpdateSection: View {
    let model: PluginsModel
    let id: String
    let enabled: Bool
    @State private var isOn = false
    @State private var busy = false
    @State private var problem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Store.storeUpdate)) {
            Toggle(isOn: Binding(get: { isOn }, set: { set($0) })) {
                Text(.Plugin3.pluginAskEveryUpdate)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
            .toggleStyle(WontonToggleStyle())
            .disabled(busy)
            if let problem { ProblemNotice(problem: problem) }
        }
        .task(id: enabled) {
            isOn = enabled
            problem = nil
        }
    }

    private func set(_ enabled: Bool) {
        busy = true
        problem = nil
        Task {
            do {
                let detail = try await model.core.setAskEveryUpdate(id: id, enabled: enabled)
                isOn = detail.askEveryUpdate ?? enabled
            } catch {
                problem = error.problem
                isOn = self.enabled
            }
            busy = false
        }
    }
}

private struct DetailHeader: View {
    let model: PluginsModel
    let detail: PluginDetail
    let action: DetailAction?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                PluginIconView(
                    icon: detail.icon,
                    size: WontonTheme.size.detailIcon,
                    model: model
                )
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    Text(detail.title)
                        .wontonText(WontonTheme.typography.sectionTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                    Text(detail.summary)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                    HStack(spacing: WontonTheme.spacing.s8) {
                        Tag(text: detail.trust.label, tint: detail.trust.tint(colors))
                        if let provider = detail.provider {
                            Text(provider)
                                .wontonText(WontonTheme.typography.caption)
                                .foregroundStyle(Color(colors.textSecondary))
                        }
                        if let writer = detail.installed?.writtenBy {
                            Text(.Plugin3.pluginWrittenByAgent(writer.title))
                                .wontonText(WontonTheme.typography.caption)
                                .foregroundStyle(Color(colors.textSecondary))
                        }
                    }
                }
            }
            if let action {
                Button(action: action.perform) {
                    Text(action.title).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(!action.enabled)
            }
            if let blocked = detail.blocked {
                WontonNotice(icon: .blocked, title: blocked)
            }
            if let installed = detail.installed, case let .degraded(reason) = installed.status {
                WontonNotice(icon: .warning, title: reason)
            }
        }
    }
}

private struct Screenshots: View {
    let model: PluginsModel
    let images: [PluginImage]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s12) {
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
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image).resizable().scaledToFit()
            } else {
                Color(colors.surfaceSubtle)
                    .aspectRatio(9 / 19.5, contentMode: .fit)
                    .overlay { WontonIcon.imageBroken.view().foregroundStyle(Color(colors.textSecondary)) }
            }
        }
        .frame(height: WontonTheme.size.screenshot)
        .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.card))
        .accessibilityLabel(String(localized: .Plugin1.pluginScreenshot))
        .task(id: source.path) { image = try? await model.image(source) }
    }
}

private struct DetailSections: View {
    let detail: PluginDetail
    let model: PluginsModel
    let updated: (PluginDetail) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let notes = detail.releaseNotes {
            DetailSection(title: String(localized: .Install.installReleaseNotes)) {
                Text(detail.version)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                Text(notes)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
        }
        if !detail.permissions.isEmpty {
            DetailSection(title: String(localized: .Install.installPermissions)) {
                ForEach(detail.permissions, id: \.permission) { line in
                    if line.optional, detail.installed != nil {
                        OptionalPermissionRow(model: model, id: detail.id, line: line, updated: updated)
                    } else {
                        PermissionRow(line: line)
                    }
                }
            }
        }
        if !detail.requirements.isEmpty {
            DetailSection(title: String(localized: .Plugin3.pluginRequirements)) {
                ForEach(detail.requirements, id: \.text) { requirement in
                    HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                        WontonIcon.check.view(size: WontonTheme.size.iconSmall)
                            .foregroundStyle(Color(colors.textSecondary))
                        Text(requirement.text)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                    }
                }
            }
        }
        DetailSection(title: String(localized: .Plugin3.pluginInfo)) {
            WontonGroup {
                ForEach(Array(info.enumerated()), id: \.offset) { index, row in
                    if index > 0 { WontonSeparator() }
                    WontonRow(title: row.title, value: row.value)
                }
                ForEach(Array(links.enumerated()), id: \.offset) { _, link in
                    WontonSeparator()
                    Link(destination: link.url) { WontonRow(title: link.title, showsForward: true) }
                        .buttonStyle(.plain)
                }
            }
        }
    }

    private var info: [(title: String, value: String)] {
        var rows = [(title: String(localized: .Plugin2.pluginInfoVersion), value: detail.version)]
        if !detail.tags.isEmpty {
            rows.append((String(localized: .Plugin0.pluginInfoTags), detail.tags.formatted(.list(type: .and))))
        }
        if let author = detail.author { rows.append((String(localized: .Plugin0.pluginInfoAuthor), author.name)) }
        if let license = detail.license { rows.append((String(localized: .Plugin0.pluginInfoLicense), license)) }
        if let source = detail.source { rows.append((String(localized: .Plugin3.pluginInfoSource), source)) }
        if let stars = detail.stars { rows.append((String(localized: .Plugin1.pluginInfoStars), stars.formatted())) }
        return rows
    }

    private var links: [(title: String, url: URL)] {
        var rows: [(title: String, url: URL)] = []
        if let link = detail.homepage.flatMap(URL.init(string:)) {
            rows.append((String(localized: .Plugin3.pluginInfoHomepage), link))
        }
        if let link = detail.repository.flatMap(URL.init(string:)) {
            rows.append((String(localized: .Plugin3.pluginInfoRepository), link))
        }
        return rows
    }
}

extension View {
    /// 卸载前确认：可以保留数据，也可以连数据一起删。
    func uninstallDialog(_ target: Binding<UninstallTarget?>, model: PluginsModel) -> some View {
        let id = target.wrappedValue?.id ?? ""
        return wontonDialog(
            String(localized: .Plugin2.pluginUninstallTitle(target.wrappedValue?.title ?? "")),
            message: String(localized: .Plugin1.pluginUninstallDeleteDataWarning),
            isPresented: Binding(
                get: { target.wrappedValue != nil },
                set: { isShowing in
                    if !isShowing { target.wrappedValue = nil }
                }
            ),
            actions: [
                WontonDialogAction(title: String(localized: .Plugin1.pluginUninstallKeepData)) {
                    model.send(.uninstallPlugin(id: id, deleteData: false))
                },
                WontonDialogAction(title: String(localized: .Plugin1.pluginUninstallDeleteData), kind: .danger) {
                    model.send(.uninstallPlugin(id: id, deleteData: true))
                },
                WontonDialogAction(title: String(localized: .Common.commonCancel))
            ]
        )
    }
}
