import SwiftUI
import WontonClient

/// 相册列表：我的、家庭、共享给我，各一组封面卡片。
struct GalleryAlbumsView: View {
    let model: GalleryModel
    let open: (PhotoAlbumRow) -> Void
    let create: () -> Void
    let openStore: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    private var albums: PhotoAlbums { model.view.albums }
    private var isEmpty: Bool { albums.mine.isEmpty && albums.household.isEmpty && albums.sharedWithMe.isEmpty }

    var body: some View {
        if isEmpty {
            if let failure = albums.problem {
                GalleryProblemView(failure: failure, retry: model.retry, openStore: openStore)
            } else if albums.loading || !model.opened {
                LoadingView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                GalleryEmptyView(icon: .folder, title: GalleryText.emptyAlbums) {
                    if model.supports(.createAlbum) {
                        Button(GalleryText.newAlbum, action: create)
                            .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                }
            }
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if let failure = albums.problem {
                        ProblemRow(problem: failure.galleryProblem, retry: model.retry)
                            .padding(.bottom, WontonTheme.spacing.s16)
                    }
                    group(GalleryText.albumsMine, albums.mine)
                    group(GalleryText.albumsHousehold, albums.household)
                    group(GalleryText.albumsShared, albums.sharedWithMe)
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
            .contentMargins(.bottom, GalleryChrome.bottomClearance, for: .scrollContent)
            .refreshable { model.retry() }
        }
    }

    @ViewBuilder private func group(_ title: String, _ rows: [PhotoAlbumRow]) -> some View {
        if !rows.isEmpty {
            Text(title)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .padding(.top, WontonTheme.spacing.s16)
                .padding(.bottom, WontonTheme.spacing.s12)
                .accessibilityAddTraits(.isHeader)
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 150), spacing: WontonTheme.spacing.s12, alignment: .top)],
                spacing: WontonTheme.spacing.s16
            ) {
                ForEach(rows, id: \.id) { row in
                    Button { open(row) } label: { GalleryAlbumCard(row: row, model: model) }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("gallery.album.\(row.id)")
                }
            }
            .padding(.bottom, WontonTheme.spacing.s8)
        }
    }
}

/// 一个相册：封面、名字、张数；别人的写明是谁的，共享着的带标记。
private struct GalleryAlbumCard: View {
    let row: PhotoAlbumRow
    let model: GalleryModel
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Color(colors.surfaceSubtle)
                .aspectRatio(1, contentMode: .fit)
                .overlay {
                    if let cover = row.cover {
                        GalleryImage(
                            request: GalleryImageRequest(
                                scope: model.cacheScope, subject: .item(id: cover, grade: .thumbnail), maxPixel: 512
                            ),
                            thumbhash: nil, core: model.core
                        )
                    } else {
                        WontonIcon.image.view(size: WontonTheme.size.taskIcon)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text(row.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                HStack(spacing: WontonTheme.spacing.s4) {
                    if row.shared {
                        WontonIcon.person.view(size: WontonTheme.size.iconTiny)
                            .accessibilityHidden(true)
                    }
                    Text(subtitle).lineLimit(1)
                }
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private var subtitle: String {
        let count = GalleryText.itemCount(Int(row.count))
        switch row.owner {
        case let .person(user): return "\(count) · \(GalleryText.from(user.name))"
        case .unknown: return "\(count) · \(GalleryText.someoneElses)"
        default: return count
        }
    }
}

/// 打开着的相册：条目的网格，翻页由核心管。是页面层上推进去的一页，和根页共用一份 `GallerySession`。
struct GalleryAlbumScreen: View {
    let session: GallerySession
    let id: String
    @AppStorage("gallery.columnLevel") private var level = GalleryGrid.defaultLevel
    @Environment(\.displayScale) private var displayScale

    private var model: GalleryModel { session.model }
    private var selection: GallerySelection { session.selection }
    private var album: PhotoAlbumView? { model.view.album.flatMap { $0.id == id ? $0 : nil } }

    var body: some View {
        GeometryReader { geometry in
            let columns = GalleryGrid.count(level: level, width: geometry.size.width)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if let album {
                        GalleryPageGrid(
                            page: album.page, origin: .album, columns: columns, model: model, selection: selection,
                            open: { session.show($0, in: .album) }, retry: { model.send(.openAlbum(id: id)) },
                            empty: { GalleryEmptyView(icon: .image, title: GalleryText.emptyAlbum) { EmptyView() } }
                        )
                    } else {
                        GalleryGridSkeleton(columns: columns)
                    }
                }
                .galleryDragSelect(selection)
            }
            .scrollDisabled(selection.dragging)
            .contentMargins(.bottom, GalleryChrome.bottomClearance, for: .scrollContent)
            .galleryCells(columns: columns, width: geometry.size.width, scale: displayScale)
            .galleryPinch(level: $level, selection: selection)
        }
        .wontonPushedScreen(title, usesSystemToolbar: true) { trailing }
        .modifier(GalleryStage(session: session, isAlbum: true))
        .onAppear {
            session.album = id
            if album == nil { model.send(.openAlbum(id: id)) }
        }
        .onDisappear {
            // 退回根页：选择收掉，核心那边的相册也关掉。
            guard session.album == id else { return }
            session.album = nil
            selection.stop()
            if model.view.album != nil { model.send(.closeAlbum) }
        }
    }

    private var title: String {
        if selection.active { return GalleryText.selectedCount(model.view.selected.count) }
        return album?.album?.title ?? ""
    }

    @ViewBuilder private var trailing: some View {
        if selection.active {
            Button(String(localized: .Common.commonDone)) { selection.stop() }
        } else if let album, !album.page.tiles.isEmpty {
            WontonIconButton(icon: .checkCircle, label: GalleryText.select) { selection.start() }
            if let row = album.album, row.owner == .me, model.supports(.shareAlbum) {
                WontonIconButton(icon: .person, label: GalleryText.actionShare) { session.flow.sharing = row }
            }
        }
    }
}
