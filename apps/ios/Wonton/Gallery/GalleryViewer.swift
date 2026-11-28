import AVKit
import SwiftUI
import WontonClient

/// 从哪一处的哪一条点开的大图页。
struct GalleryViewerTarget: Equatable {
    let origin: GalleryOrigin
    let start: String
}

/// 大图页：左右滑着看同一处的条目，双击和捏合缩放，下滑关闭；视频直接播，实况照片按住播放。
/// 看的是哪一条由这里告诉核心（`OpenDetail`），详情由核心取回来。
struct GalleryViewer: View {
    let target: GalleryViewerTarget
    let model: GalleryModel
    let flow: GalleryEditFlow
    let close: () -> Void
    @State private var current: String?
    @State private var zoomed = false
    @State private var showsChrome = true
    @State private var drag = CGSize.zero
    @State private var showsInfo = false
    @State private var motion = GalleryMotion()

    init(target: GalleryViewerTarget, model: GalleryModel, flow: GalleryEditFlow, close: @escaping () -> Void) {
        self.target = target
        self.model = model
        self.flow = flow
        self.close = close
        _current = State(initialValue: target.start)
    }

    private var tiles: [PhotoTile] { model.tiles(in: target.origin) }
    private var tile: PhotoTile? { tiles.first { $0.id == current } }
    private var progress: CGFloat { min(max(drag.height / 320, 0), 1) }

    var body: some View {
        ZStack {
            Color.black.opacity(1 - progress * 0.85).ignoresSafeArea()
            pager
                .offset(drag)
                .scaleEffect(1 - progress * 0.25)
            if showsChrome, drag == .zero, let tile {
                GalleryViewerChrome(
                    tile: tile, model: model, flow: flow, motion: motion,
                    close: close, info: { showsInfo = true }
                )
                .transition(.opacity)
            }
        }
        .environment(\.colorScheme, .dark)
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape, close)
        .onAppear { if let current { model.send(.openDetail(id: current)) } }
        .onChange(of: current) { _, id in
            motion.stop()
            zoomed = false
            guard let id else { return }
            model.send(.openDetail(id: id))
            if tiles.suffix(4).contains(where: { $0.id == id }) { model.loadMore(in: target.origin) }
        }
        .onChange(of: tiles.map(\.id)) { old, ids in
            // 正看着的这一条被移走了（进回收站、移出相册）：接着看它后面那条，没有了就关掉。
            guard let current, !ids.contains(current) else { return }
            let index = old.firstIndex(of: current) ?? 0
            if ids.isEmpty { close() } else { self.current = ids[min(index, ids.count - 1)] }
        }
        .sheet(isPresented: $showsInfo) {
            if let tile {
                GalleryInfoPanel(
                    detail: model.view.detail.flatMap { $0.id == tile.id ? $0 : nil }, tile: tile,
                    retry: { model.send(.openDetail(id: tile.id)) }
                )
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
            }
        }
    }

    private var pager: some View {
        ScrollView(.horizontal) {
            LazyHStack(spacing: 0) {
                ForEach(tiles, id: \.id) { tile in
                    GalleryViewerPage(
                        tile: tile, model: model, active: tile.id == current,
                        zoomed: $zoomed, motion: motion, handle: handle
                    )
                    .containerRelativeFrame([.horizontal, .vertical])
                }
            }
            .scrollTargetLayout()
        }
        .scrollTargetBehavior(.paging)
        .scrollPosition(id: $current)
        .scrollIndicators(.hidden)
        .scrollDisabled(zoomed || drag != .zero)
        .ignoresSafeArea()
    }

    private func handle(_ event: GalleryZoomEvent) {
        switch event {
        case .tap:
            withAnimation(.easeOut(duration: WontonTheme.motion.state)) { showsChrome.toggle() }
        case let .dismissDrag(translation):
            drag = translation
        case let .dismissEnd(commit):
            if commit {
                close()
            } else {
                withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) { drag = .zero }
            }
        case let .hold(pressing):
            guard let tile, tile.live, !tile.isVideo else { return }
            if pressing {
                motion.play(
                    id: tile.id, rendition: .live, core: model.core, muted: false, title: GalleryText.liveFailed
                )
            } else {
                motion.release()
            }
        }
    }
}

/// 大图页的一页：先是网格里那张小图或占位，预览图到了换上；视频在这一页上播。
private struct GalleryViewerPage: View {
    let tile: PhotoTile
    let model: GalleryModel
    let active: Bool
    @Binding var zoomed: Bool
    let motion: GalleryMotion
    let handle: (GalleryZoomEvent) -> Void
    @State private var image: UIImage?
    @State private var failure: Problem?
    @State private var attempt = 0
    @Environment(\.galleryCellPixel) private var cellPixel

    private func request(_ grade: GalleryImageRequest.Grade, _ pixel: Int) -> GalleryImageRequest {
        GalleryImageRequest(scope: model.cacheScope, subject: .item(id: tile.id, grade: grade), maxPixel: pixel)
    }

    var body: some View {
        ZStack {
            if let image {
                GalleryZoomView(image: image, zoomed: active ? $zoomed : .constant(false), handle: handle)
            } else {
                placeholder
            }
            if active, let player = motion.player {
                if tile.isVideo {
                    VideoPlayer(player: player)
                } else {
                    GalleryPlayerLayer(player: player)
                }
            } else if active, tile.isVideo {
                playControl
            }
            if let failure {
                VStack {
                    Spacer()
                    ProblemRow(problem: failure, retry: { attempt += 1 })
                        .padding(WontonTheme.spacing.s12)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.card))
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .padding(.bottom, 140)
                }
            }
        }
        .task(id: attempt) { await load() }
        .onChange(of: active, initial: true) { _, active in
            // 滑到视频这一页就开播，滑走由大图页停。
            if active, tile.isVideo { play() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("gallery.viewer.\(tile.id)")
    }

    @ViewBuilder private var placeholder: some View {
        if let hash = tile.thumbhash.flatMap(ThumbHash.image) {
            Image(uiImage: hash).resizable().aspectRatio(tile.aspect.map { CGFloat($0) }, contentMode: .fit)
        }
        if failure == nil { ProgressView().tint(.white) }
    }

    @ViewBuilder private var playControl: some View {
        if motion.isLoading {
            ProgressView().tint(.white).controlSize(.large)
        } else {
            Button(action: play) {
                WontonIcon.playFill.view(size: WontonTheme.size.icon)
                    .foregroundStyle(.white)
                    .environment(\.wontonIconMono, true)
                    .frame(width: 64, height: 64)
                    .background(.ultraThinMaterial, in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(GalleryText.play)
        }
    }

    private func play() {
        motion.play(
            id: tile.id, rendition: .playback, core: model.core, muted: false, title: GalleryText.videoFailed
        )
    }

    private func load() async {
        failure = nil
        let pipeline = GalleryImagePipeline.shared
        if image == nil { image = pipeline.cached(request(.thumbnail, cellPixel)) }
        switch await pipeline.image(request(.preview, 2560), core: model.core) {
        case let .success(preview):
            image = preview
        case let .failure(reason):
            if !Task.isCancelled { failure = reason.problem }
        }
    }
}

/// 大图页上盖着的那一层：返回、时间、信息，底下是这个源接的操作；点一下照片收起或露出。
private struct GalleryViewerChrome: View {
    let tile: PhotoTile
    let model: GalleryModel
    let flow: GalleryEditFlow
    let motion: GalleryMotion
    let close: () -> Void
    let info: () -> Void

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            HStack(alignment: .top) {
                GalleryPhotoButton(icon: .back, label: String(localized: .Design.designBack), action: close)
                    .accessibilityIdentifier("gallery.viewer.close")
                Spacer(minLength: WontonTheme.spacing.s8)
                VStack(spacing: 2) {
                    Text(tile.takenDate.formatted(.dateTime.year().month().day()))
                        .wontonText(WontonTheme.typography.barTitle)
                    Text(tile.takenDate.formatted(date: .omitted, time: .shortened))
                        .wontonText(WontonTheme.typography.barStatus)
                        .opacity(0.8)
                }
                .foregroundStyle(.white)
                .shadow(color: .black.opacity(0.5), radius: 3)
                .accessibilityElement(children: .combine)
                Spacer(minLength: WontonTheme.spacing.s8)
                GalleryPhotoButton(icon: .info, label: GalleryText.actionInfo, action: info)
                    .accessibilityIdentifier("gallery.viewer.info")
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                if tile.live, !tile.isVideo { badge(GalleryText.live) }
                if let owner = tile.ownerLine { badge(owner) }
                Spacer(minLength: 0)
            }
            Spacer(minLength: 0)
            if let problem = motion.problem {
                ProblemNotice(problem: problem, onClose: motion.stop)
            }
            if !actions.isEmpty {
                GalleryActionBar(actions: actions, busy: model.view.editing > 0, onPhoto: true)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
    }

    private func badge(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, WontonTheme.spacing.s8)
            .padding(.vertical, WontonTheme.spacing.s4)
            .background(.black.opacity(0.45), in: Capsule())
    }

    private var actions: [GalleryAction] {
        var list: [GalleryAction] = []
        let ids = [tile.id]
        if tile.favorite, model.supports(.unfavorite) {
            list.append(GalleryAction(id: "unfavorite", icon: .starFill, title: GalleryText.actionUnfavorite) {
                flow.favorite(ids, false)
            })
        } else if !tile.favorite, model.supports(.favorite) {
            list.append(GalleryAction(id: "favorite", icon: .star, title: GalleryText.actionFavorite) {
                flow.favorite(ids, true)
            })
        }
        if model.supports(.addToAlbum) || model.supports(.createAlbum) {
            list.append(GalleryAction(id: "addToAlbum", icon: .folder, title: GalleryText.actionAddToAlbum) {
                flow.picking = ids
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
