import SwiftUI
import WontonClient

/// 点开相册插件的那一页（`TabPage.native` 是相册，ADR-0113 第 9 节）：原生的相册页，右上角“更多”里有网页版、
/// 加进我的应用和详情。
struct GalleryLibraryRoot: View {
    let tab: TabRef
    let pluginId: String
    let title: String
    let pages: PagesModel
    let plugins: PluginsModel
    let chat: ChatModel
    let open: (PluginEntry) -> Void
    let openStore: () -> Void
    /// 往页面层推一个相册。
    let push: (GalleryAlbumRoute) -> Void
    @State private var showsDetails = false

    var body: some View {
        GalleryRootView(
            session: GallerySession.of(core: pages.core, source: pluginId), title: title,
            openStore: openStore,
            openAlbum: { push(GalleryAlbumRoute(pluginId: pluginId, albumId: $0)) },
            actions: GalleryRootActions(
                openWeb: { pages.open(.pluginPath(pluginId: pluginId, path: "/")) },
                addToMyApps: {
                    pages.send(.pages(action: .addMyApp(app: .page(tab: tab), title: nil, index: nil)))
                },
                details: { showsDetails = true }
            )
        )
        .sheet(isPresented: $showsDetails) {
            NavigationStack {
                PluginDetailView(model: plugins, chat: chat, route: .installed(id: pluginId)) { entry in
                    showsDetails = false
                    open(entry)
                }
            }
        }
    }
}

/// 根页右上角“更多”里的三件事。
struct GalleryRootActions {
    let openWeb: () -> Void
    let addToMyApps: () -> Void
    let details: () -> Void
}

/// 原生相册页的根：时间线、相册、搜索三个分区。只画核心给的视图、把操作发回核心。
/// 用哪个相册插件先看核心定下来没有（`PhotosViewModel.sources`），定下来才有这三个分区。
struct GalleryRootView: View {
    let session: GallerySession
    let title: String
    let openStore: () -> Void
    let openAlbum: (String) -> Void
    let actions: GalleryRootActions
    @AppStorage("gallery.columnLevel") private var level = GalleryGrid.defaultLevel
    @Environment(\.displayScale) private var displayScale

    private var model: GalleryModel { session.model }
    private var selection: GallerySelection { session.selection }
    private var ready: Bool { model.view.sources.state == .ready }

    var body: some View {
        content
            .wontonPushedScreen(
                selection.active ? GalleryText.selectedCount(model.view.selected.count) : title,
                usesSystemToolbar: true
            ) { trailing }
            .modifier(GalleryStage(session: session, isAlbum: false))
            .onChange(of: session.section) { _, _ in selection.stop() }
    }

    @ViewBuilder private var content: some View {
        let sources = model.view.sources
        switch sources.state {
        case .ready:
            sections
        case .checking:
            ScrollView { GalleryGridSkeleton(columns: GalleryGrid.levels[GalleryGrid.defaultLevel]) }
                .scrollDisabled(true)
        case .notInstalled:
            // 没有能用的相册应用不是出错：照实说，给一个去装的出口。
            GalleryEmptyView(icon: .image, title: GalleryText.sourceNotInstalled) {
                Button(GalleryText.openStore, action: openStore)
                    .buttonStyle(WontonButtonStyle(kind: .primary))
            }
            .accessibilityIdentifier("gallery.source.notInstalled")
        case .choose:
            GallerySourceList(sources: sources.candidates) { model.send(.chooseSource(id: $0)) }
        case .failed:
            if let failure = sources.problem {
                GalleryProblemView(failure: failure, retry: model.retry)
            } else {
                GalleryEmptyView(icon: .warning, title: GalleryText.problemGenericTitle) {
                    Button(String(localized: .Common.commonRetry), action: model.retry)
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        }
    }

    private var sections: some View {
        @Bindable var session = session
        return GeometryReader { geometry in
            let columns = GalleryGrid.count(level: level, width: geometry.size.width)
            Group {
                switch session.section {
                case .timeline:
                    GalleryTimelineView(
                        model: model, selection: selection, columns: columns,
                        open: { session.show($0, in: .timeline) }, openStore: openStore
                    )
                case .albums:
                    GalleryAlbumsView(
                        model: model, open: { openAlbum($0.id) },
                        create: { session.flow.startNaming(with: []) }, openStore: openStore
                    )
                case .search:
                    GallerySearchView(
                        model: model, selection: selection, columns: columns, draft: $session.draft,
                        open: { session.show($0, in: .search) }
                    )
                }
            }
            .galleryCells(columns: columns, width: geometry.size.width, scale: displayScale)
            .galleryPinch(level: $level, selection: selection)
        }
    }

    @ViewBuilder private var trailing: some View {
        if selection.active {
            Button(String(localized: .Common.commonDone)) { selection.stop() }
                .accessibilityIdentifier("gallery.select.done")
        } else {
            if ready, session.section == .albums, model.supports(.createAlbum) {
                WontonIconButton(icon: .add, label: GalleryText.newAlbum) { session.flow.startNaming(with: []) }
            }
            if ready, session.section != .albums, !model.tiles(in: session.origin).isEmpty {
                WontonIconButton(icon: .checkCircle, label: GalleryText.select) { selection.start() }
                    .accessibilityIdentifier("gallery.select")
            }
            Menu {
                Button(GalleryText.webVersion, systemImage: "globe", action: actions.openWeb)
                Button(TabText.addToMyApps, systemImage: "plus.square.on.square", action: actions.addToMyApps)
                Button(TabText.details, systemImage: "info.circle", action: actions.details)
            } label: {
                WontonIcon.more.view()
            }
            .accessibilityLabel(TabText.more)
            .accessibilityIdentifier("gallery.more")
        }
    }
}

/// 有几家相册应用、主人没定默认、这个人也没挑过：列出来挑一家，核心记住（`PhotosAction.ChooseSource`）。
private struct GallerySourceList: View {
    let sources: [PhotoSource]
    let choose: (String) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                Text(GalleryText.sourceChoose)
                    .wontonText(WontonTheme.typography.sectionTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .accessibilityAddTraits(.isHeader)
                WontonGroup {
                    ForEach(Array(sources.enumerated()), id: \.element.id) { index, source in
                        if index > 0 { WontonSeparator() }
                        Button { choose(source.id) } label: {
                            HStack(spacing: WontonTheme.spacing.s12) {
                                WontonIcon.image.view().foregroundStyle(Color(colors.textSecondary))
                                Text(source.title)
                                    .wontonText(WontonTheme.typography.body)
                                    .foregroundStyle(Color(colors.textPrimary))
                                Spacer(minLength: 0)
                                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                                    .foregroundStyle(Color(colors.textSecondary))
                            }
                            .padding(.horizontal, WontonTheme.spacing.s16)
                            .frame(minHeight: WontonTheme.size.target + WontonTheme.spacing.s8)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("gallery.source.\(source.id)")
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }
}

/// 根页和相册页共用的那一层：底下悬浮的分区条或操作条、写操作的结果提示、盖在上面的大图页，和写操作的面板。
/// 两页都在页面层上，只有露在最上面的那一页画这一层。
struct GalleryStage: ViewModifier {
    let session: GallerySession
    let isAlbum: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var model: GalleryModel { session.model }
    private var selection: GallerySelection { session.selection }
    private var isTop: Bool { isAlbum == (session.album != nil) }
    private var viewer: GalleryViewerTarget? {
        guard let viewer = session.viewer, (viewer.origin == .album) == isAlbum else { return nil }
        return viewer
    }

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .bottom) { floating }
            .overlay {
                if let viewer {
                    GalleryViewer(target: viewer, model: model, flow: session.flow, close: session.closeViewer)
                        .overlay(alignment: .bottom) {
                            // 大图页自己底下有一条操作，提示放在它上面。
                            notice.padding(.bottom, GalleryChrome.bottomClearance)
                        }
                        .transition(reduceMotion ? .opacity : .opacity.combined(with: .scale(scale: 0.94)))
                }
            }
            // 大图页盖住整页：系统栏收起，左边缘右滑也先不退页（`suppressesEdgePop`）。
            .toolbar(viewer == nil ? .automatic : .hidden, for: .navigationBar)
            .onChange(of: viewer != nil, initial: true) { _, open in
                if isTop { UINavigationController.suppressesEdgePop = open }
            }
            .modifier(GalleryEditSheets(flow: session.flow, model: model, enabled: isTop))
            .wontonDialog(problem: model.rejection, isPresented: Binding(
                get: { isTop && model.rejection != nil }, set: { if !$0 { model.rejection = nil } }
            ))
            .animation(.easeOut(duration: WontonTheme.motion.state), value: model.view.lastEdit != nil)
            .sensoryFeedback(.selection, trigger: model.view.selected.count)
            .onChange(of: session.origin, initial: true) { _, origin in
                selection.orderSource = { [model] in model.tiles(in: origin).map(\.id) }
            }
            .onChange(of: model.view.selected.isEmpty) { _, empty in
                // 选中的都处理完了（收藏、移走）：退出选择，不留一个“已选 0 项”。
                if isTop, empty, model.view.lastEdit != nil { selection.stop() }
            }
            .onDisappear {
                if isTop { UINavigationController.suppressesEdgePop = false }
            }
    }

    @ViewBuilder private var floating: some View {
        if viewer == nil, isTop {
            VStack(spacing: WontonTheme.spacing.s8) {
                notice
                bottomBar
            }
            .padding(.bottom, WontonTheme.spacing.s8)
        }
    }

    @ViewBuilder private var notice: some View {
        if let outcome = model.view.lastEdit {
            GalleryEditNotice(
                outcome: outcome, canUndo: model.supports(.restore),
                undo: { session.flow.undoTrash(outcome.done) },
                dismiss: { model.send(.dismissEditOutcome) }
            )
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }

    @ViewBuilder private var bottomBar: some View {
        @Bindable var session = session
        if selection.active {
            let actions = selectionActions
            if !actions.isEmpty, !model.view.selected.isEmpty {
                GalleryActionBar(actions: actions, busy: model.view.editing > 0)
            }
        } else if !isAlbum, model.view.sources.state == .ready {
            GallerySectionBar(section: $session.section)
        }
    }

    /// 多选时的操作：只列这个源接的；在相册里多一个“移出相册”。
    private var selectionActions: [GalleryAction] {
        let ids = model.view.selected
        let flow = session.flow
        let picked = model.tiles(in: session.origin).filter { ids.contains($0.id) }
        var list: [GalleryAction] = []
        let allFavorite = !picked.isEmpty && picked.allSatisfy(\.favorite)
        if allFavorite, model.supports(.unfavorite) {
            list.append(GalleryAction(id: "unfavorite", icon: .starFill, title: GalleryText.actionUnfavorite) {
                flow.favorite(ids, false)
            })
        } else if !allFavorite, model.supports(.favorite) {
            list.append(GalleryAction(id: "favorite", icon: .star, title: GalleryText.actionFavorite) {
                flow.favorite(ids, true)
            })
        }
        if model.supports(.addToAlbum) || model.supports(.createAlbum) {
            list.append(GalleryAction(id: "addToAlbum", icon: .folder, title: GalleryText.actionAddToAlbum) {
                flow.picking = ids
            })
        }
        if isAlbum, let album = model.view.album?.id, model.supports(.removeFromAlbum) {
            list.append(GalleryAction(id: "removeFromAlbum", icon: .close, title: GalleryText.actionRemoveFromAlbum) {
                flow.remove(ids, from: album)
            })
        }
        if model.supports(.trash) {
            list.append(GalleryAction(id: "trash", icon: .trash, title: GalleryText.actionTrash, isDanger: true) {
                flow.trash(ids)
            })
        }
        return list
    }
}
