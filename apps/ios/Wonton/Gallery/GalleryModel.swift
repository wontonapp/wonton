import Foundation
import Observation
import SwiftUI
import WontonClient

/// 网格的来处：大图页左右滑时沿着同一处的条目走。
enum GalleryOrigin: Hashable {
    case timeline
    case album
    case search
}

/// 持有核心推来的相册页，把用户操作转发给核心（ADR-0113 第 9 节）。游标、按日分组和选择集都在核心。
@MainActor
@Observable
final class GalleryModel {
    private(set) var view: PhotosViewModel
    /// 核心当场拒绝的操作，弹窗说明。
    var rejection: Problem?
    /// 进过相册页了：之前的空视图不是“没有照片”，先画骨架。
    private(set) var opened = false
    /// 用哪个相册插件（插件 id）：从插件的原生页进来时是那个插件；空着由核心按默认挑。
    private(set) var source: String?
    let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        view = core.photos()
    }

    /// 订阅核心的相册页，一直到任务被取消。
    func observe() async {
        for await next in core.subscribePhotos().updates() {
            view = next
        }
    }

    func send(_ action: PhotosAction) {
        do {
            try core.dispatch(action: .photos(action: action))
        } catch {
            rejection = error.problem
        }
        view = core.photos()
    }

    /// 进相册页。时差按手机此刻的算，核心用它按日分组。
    func open(source: String? = nil) {
        if let source { self.source = source }
        opened = true
        let minutes = Int32(TimeZone.current.secondsFromGMT() / 60)
        send(.open(utcOffsetMinutes: minutes, source: self.source))
    }

    func retry() {
        send(.refresh)
    }

    /// 图片缓存的作用域：服务器加相册插件。条目 id 只在这个范围里唯一。
    var cacheScope: String {
        "\(core.home().serverId ?? "none")/\(source ?? "default")"
    }

    var selected: Set<String> { Set(view.selected) }

    func supports(_ kind: PhotoEditKind) -> Bool {
        view.actions?.contains(kind) ?? false
    }

    func tiles(in origin: GalleryOrigin) -> [PhotoTile] {
        switch origin {
        case .timeline: view.timeline.months.flatMap { $0.days.flatMap(\.tiles) }
        case .album: view.album?.page.tiles ?? []
        case .search: view.search?.page.tiles ?? []
        }
    }

    /// 走到这一处的末尾附近：还有下一页就取。
    func loadMore(in origin: GalleryOrigin) {
        switch origin {
        case .timeline:
            if let month = view.timeline.months.first(where: { !$0.opened }) {
                send(.openMonth(month: month.month))
            } else if let month = view.timeline.months.first(where: { $0.hasMore && !$0.loadingMore }) {
                send(.loadMoreMonth(month: month.month))
            }
        case .album:
            if let page = view.album?.page, page.hasMore, !page.loadingMore { send(.loadMoreAlbum) }
        case .search:
            if let page = view.search?.page, page.hasMore, !page.loadingMore { send(.loadMoreSearch) }
        }
    }
}

extension PhotoTile {
    /// 别人的条目标谁的：家人写名字；不是这台服务器上的家人就没有名字可写，只说是别人的。自己的不标。
    var ownerBadge: String? {
        switch owner {
        case .me: nil
        case let .person(user): user.name
        case .unknown: GalleryText.someoneElses
        }
    }

    /// 同上，写成一句“来自谁”。
    var ownerLine: String? {
        switch owner {
        case .me: nil
        case let .person(user): GalleryText.from(user.name)
        case .unknown: GalleryText.someoneElses
        }
    }
    var takenDate: Date { Date(timeIntervalSince1970: TimeInterval(takenAt) / 1000) }
    var isVideo: Bool { kind == .video }
}

/// 相册里推进去的一个相册：哪个插件的哪个相册。
struct GalleryAlbumRoute: Hashable {
    let pluginId: String
    let albumId: String
}

/// 一个相册插件的原生页在壳里的那点状态：根页和推进去的相册页是页面层上的两页，共用一份。
/// 订阅跟着它活；换了相册插件就换一份。
@MainActor
@Observable
final class GallerySession {
    let model: GalleryModel
    let selection: GallerySelection
    let flow: GalleryEditFlow
    var section = GallerySection.timeline
    var draft = GallerySearchDraft()
    /// 开着的大图页。
    var viewer: GalleryViewerTarget?
    /// 推进去的相册；空着是在根页。
    var album: String?
    @ObservationIgnored private var observation: Task<Void, Never>?
    private static var current: (source: String, session: GallerySession)?

    private init(core: ClientCore, source: String) {
        let model = GalleryModel(core: core)
        self.model = model
        selection = GallerySelection(model: model)
        flow = GalleryEditFlow(model: model)
        observation = Task { await model.observe() }
        model.open(source: source)
    }

    deinit { observation?.cancel() }

    static func of(core: ClientCore, source: String) -> GallerySession {
        if let current, current.source == source { return current.session }
        let session = GallerySession(core: core, source: source)
        current = (source, session)
        return session
    }

    /// 正看着的这一处。
    var origin: GalleryOrigin {
        album != nil ? .album : (section == .search ? .search : .timeline)
    }

    func show(_ tile: PhotoTile, in origin: GalleryOrigin) {
        withAnimation(.easeOut(duration: WontonTheme.motion.panel)) {
            viewer = GalleryViewerTarget(origin: origin, start: tile.id)
        }
    }

    func closeViewer() {
        withAnimation(.easeOut(duration: WontonTheme.motion.panel)) { viewer = nil }
        model.send(.closeDetail)
    }
}
