import Observation
import SwiftUI
import WontonClient

/// 网格的排法：正方形格子、细缝，列数由捏合换档。
enum GalleryGrid {
    static let gap: CGFloat = 2
    static let space = "galleryGrid"
    /// 捏合的四档，数字是手机竖屏的列数；宽屏按宽度等比多排。
    static let levels = [2, 3, 4, 6]
    static let defaultLevel = 2

    static func columns(_ count: Int) -> [GridItem] {
        Array(repeating: GridItem(.flexible(), spacing: gap), count: max(1, count))
    }

    static func count(level: Int, width: CGFloat) -> Int {
        let base = levels[min(max(level, 0), levels.count - 1)]
        return max(base, Int((CGFloat(base) * width / 400).rounded()))
    }
}

/// 多选的手势状态：长按进入选择，手不抬起接着拖就连选。选中了哪些在核心（`PhotosViewModel.selected`），
/// 这里只记格子的位置和这一次拖动的起点。
@MainActor
@Observable
final class GallerySelection {
    /// 选择模式开着：点格子是选，不是看大图。
    private(set) var active = false
    /// 正在拖动连选：这时网格不滚。
    private(set) var dragging = false
    @ObservationIgnored private var frames: [String: CGRect] = [:]
    @ObservationIgnored private var anchor: String?
    @ObservationIgnored private var lastHit: String?
    @ObservationIgnored private var base: Set<String> = []
    @ObservationIgnored private var order: [String] = []
    @ObservationIgnored private var pressEnded: Date?
    /// 正看着的这一处从头到尾的条目 id（时间线、相册或搜索结果），页面换了由宿主换掉。
    @ObservationIgnored var orderSource: () -> [String] = { [] }
    private let model: GalleryModel

    init(model: GalleryModel) {
        self.model = model
    }

    func start() {
        active = true
    }

    func stop() {
        active = false
        dragging = false
        anchor = nil
        if !model.view.selected.isEmpty { model.send(.clearSelection) }
    }

    func toggle(_ id: String) {
        // 长按刚松手的那一下也会被当成点按，不算：不然刚选上的那格又被点掉。
        if let pressEnded, Date.now.timeIntervalSince(pressEnded) < 0.35 { return }
        model.send(.select(ids: [id], selected: !model.view.selected.contains(id)))
    }

    /// 长按落在哪一格：进入选择并选中它，手不抬起接着拖就按这一处的先后连选。
    func press(at point: CGPoint) {
        guard let id = frames.first(where: { $0.value.contains(point) })?.key else { return }
        active = true
        dragging = true
        anchor = id
        lastHit = id
        order = orderSource()
        base = Set(model.view.selected)
        if !base.contains(id) { model.send(.select(ids: [id], selected: true)) }
    }

    /// 只认长按、不跟拖动的系统上：进入选择并选中这一格。
    func press(_ id: String) {
        active = true
        pressEnded = .now
        if !model.view.selected.contains(id) { model.send(.select(ids: [id], selected: true)) }
    }

    func place(_ id: String, at frame: CGRect) {
        frames[id] = frame
    }

    /// 列数变了，格子都挪了位置。
    func forgetFrames() {
        frames.removeAll()
    }

    func dragMoved(to point: CGPoint) {
        guard dragging, let anchor,
              let hit = frames.first(where: { $0.value.contains(point) })?.key, hit != lastHit,
              let from = order.firstIndex(of: anchor), let upTo = order.firstIndex(of: hit)
        else { return }
        lastHit = hit
        let target = base.union(order[min(from, upTo) ... max(from, upTo)])
        let current = Set(model.view.selected)
        let added = order.filter { target.contains($0) && !current.contains($0) }
        let removed = current.subtracting(target)
        if !added.isEmpty { model.send(.select(ids: added, selected: true)) }
        if !removed.isEmpty { model.send(.select(ids: Array(removed), selected: false)) }
    }

    func dragEnded() {
        if dragging { pressEnded = .now }
        dragging = false
        anchor = nil
        lastHit = nil
    }
}

/// 一块连续的格子（一天、一个相册、一次搜索的结果）。点和长按交给宿主。
struct GalleryTiles: View {
    let tiles: [PhotoTile]
    let columns: Int
    let model: GalleryModel
    let selection: GallerySelection
    /// 点开看大图；选择模式下点按是选。
    let open: (PhotoTile) -> Void
    @Environment(\.galleryCellPixel) private var pixel

    var body: some View {
        let selected = model.selected
        LazyVGrid(columns: GalleryGrid.columns(columns), spacing: GalleryGrid.gap) {
            ForEach(tiles, id: \.id) { tile in
                GalleryTile(
                    tile: tile, scope: model.cacheScope, core: model.core, pixel: pixel,
                    selecting: selection.active, selected: selected.contains(tile.id)
                )
                .onGeometryChange(for: CGRect.self) { proxy in
                    proxy.frame(in: .named(GalleryGrid.space))
                } action: { frame in
                    selection.place(tile.id, at: frame)
                }
                .onTapGesture {
                    if selection.active { selection.toggle(tile.id) } else { open(tile) }
                }
                .modifier(GalleryTilePress(selection: selection, id: tile.id))
            }
        }
    }
}

/// iOS 17 没有能接进 SwiftUI 的长按拖动识别器：格子自己认长按，进入选择后一格格点。
private struct GalleryTilePress: ViewModifier {
    let selection: GallerySelection
    let id: String

    func body(content: Content) -> some View {
        if #available(iOS 18, *) {
            content
        } else {
            content.onLongPressGesture(minimumDuration: 0.3) { selection.press(id) }
        }
    }
}

extension EnvironmentValues {
    /// 格子解码的像素边长，页面按宽度和列数算好交下来。
    @Entry var galleryCellPixel: Int = 256
}

extension View {
    /// 页面量出宽度后定列数和格子的解码大小。
    func galleryCells(columns: Int, width: CGFloat, scale: CGFloat) -> some View {
        let pixel = Int((width / CGFloat(max(columns, 1)) * scale).rounded(.up))
        return environment(\.galleryCellPixel, min(max(pixel, 160), 512))
    }

    /// 网格的滚动内容：给格子一个不随滚动变的坐标系，长按后的拖动在里面找手指下的那一格。
    func galleryDragSelect(_ selection: GallerySelection) -> some View {
        modifier(GalleryDragSelect(selection: selection))
    }
}

private struct GalleryDragSelect: ViewModifier {
    let selection: GallerySelection

    func body(content: Content) -> some View {
        if #available(iOS 18, *) {
            content
                .coordinateSpace(.named(GalleryGrid.space))
                .gesture(GalleryPressDrag(selection: selection))
        } else {
            content
        }
    }
}

/// 长按后不抬手接着拖：系统的长按识别器本来就这样报位置。按住不动才认，滚动照常；认出来以后网格停住不滚
/// （`GallerySelection.dragging`），手指扫过的格子连着选上。
@available(iOS 18, *)
private struct GalleryPressDrag: UIGestureRecognizerRepresentable {
    let selection: GallerySelection

    func makeUIGestureRecognizer(context: Context) -> UILongPressGestureRecognizer {
        let recognizer = UILongPressGestureRecognizer()
        recognizer.minimumPressDuration = 0.3
        return recognizer
    }

    func handleUIGestureRecognizerAction(_ recognizer: UILongPressGestureRecognizer, context: Context) {
        let point = context.converter.location(in: .named(GalleryGrid.space))
        switch recognizer.state {
        case .began: selection.press(at: point)
        case .changed: selection.dragMoved(to: point)
        case .ended, .cancelled, .failed: selection.dragEnded()
        default: break
        }
    }
}

/// 一页页的条目（相册、搜索结果）：网格、翻页、空和取不到。
struct GalleryPageGrid<Empty: View>: View {
    let page: PhotoPage
    let origin: GalleryOrigin
    let columns: Int
    let model: GalleryModel
    let selection: GallerySelection
    let open: (PhotoTile) -> Void
    let retry: () -> Void
    @ViewBuilder let empty: Empty

    var body: some View {
        if page.tiles.isEmpty {
            if let failure = page.problem {
                GalleryProblemView(failure: failure, retry: retry).padding(.top, WontonTheme.spacing.s48)
            } else if page.loading {
                GalleryGridSkeleton(columns: columns)
            } else {
                empty.padding(.top, WontonTheme.spacing.s48)
            }
        } else {
            GalleryTiles(tiles: page.tiles, columns: columns, model: model, selection: selection, open: open)
            if let failure = page.problem {
                ProblemRow(problem: failure.galleryProblem, retry: { model.loadMore(in: origin) })
                    .padding(WontonTheme.spacing.pageInset)
            } else if page.hasMore {
                ProgressView()
                    .frame(maxWidth: .infinity)
                    .padding(WontonTheme.spacing.s16)
                    .onAppear { model.loadMore(in: origin) }
            }
        }
    }
}
