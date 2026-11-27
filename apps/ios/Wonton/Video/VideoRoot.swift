import SwiftUI
import WontonClient

/// 影视页里能去的地方：海报墙（一个库、一个类型、一部剧的季、搜索）和详情。
enum VideoRoute: Hashable {
    case wall(VideoWallQuery, title: String)
    case detail(id: String, source: String?)
}

/// 一页内容的三种样子。
enum VideoLoad<Value> {
    case loading
    case loaded(Value)
    case failed(Problem)
}

/// 首页以外的一页。
struct VideoScreen: View {
    let model: VideoModel
    let route: VideoRoute

    var body: some View {
        switch route {
        case let .wall(query, title):
            VideoWallView(model: model, query: query, title: title)
        case let .detail(id, source):
            VideoDetailView(model: model, id: id, source: source)
        }
    }
}

/// 去下一页的动作。包成结构体是因为 `@Entry` 直接存闭包会告警。
struct VideoPush {
    let action: (VideoRoute) -> Void

    func callAsFunction(_ route: VideoRoute) { action(route) }
}

extension EnvironmentValues {
    @Entry var videoPush = VideoPush { _ in }
}

/// 点了去下一页的卡或按钮。
struct VideoLink<Label: View>: View {
    let route: VideoRoute
    @ViewBuilder let label: Label
    @Environment(\.videoPush) private var push

    var body: some View {
        Button { push(route) } label: { label }
    }
}

extension View {
    /// 影视页里的一页：往里点推到哪、图片缓存的作用域，和点了没成的动作弹的对话框。几页叠着时只有最上面那页
    /// （`front`）弹，同一个模型的原因不弹两遍。
    func videoPage(model: VideoModel, front: Bool, push: @escaping (VideoRoute) -> Void) -> some View {
        environment(\.videoPush, VideoPush(action: push))
            .environment(\.videoCacheScope, model.core.videoCacheScope())
            .wontonDialog(problem: model.actionProblem, isPresented: Binding(
                get: { front && model.actionProblem != nil },
                set: { if !$0 { model.actionProblem = nil } }
            ))
    }
}

extension VideoEntry {
    /// 点卡去哪：单集、电影直接进详情；剧和季也进详情（里面是季和集）；文件夹进海报墙。
    var route: VideoRoute {
        switch kind {
        case .folder:
            .wall(VideoWallQuery(
                library: nil, parent: id, kind: nil, genre: nil, search: nil, sort: .title,
                unplayedOnly: false, cursor: nil, source: source
            ), title: title)
        default:
            .detail(id: id, source: source)
        }
    }
}

extension VideoWallQuery {
    static func library(_ library: VideoLibrary, source: String?) -> VideoWallQuery {
        VideoWallQuery(
            library: library.id, parent: nil, kind: nil, genre: nil, search: nil, sort: .title,
            unplayedOnly: false, cursor: nil, source: source
        )
    }
}

/// 首页：顶上一排库，下面继续看、下一集（横版剧照），最近添加（竖版海报）。某一排没取到，顶上一条轻提示。
struct VideoHomeView: View {
    let model: VideoModel
    let root: VideoRootActions
    @State private var load: VideoLoad<VideoHome> = .loading
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .background(Color(colors.canvas).ignoresSafeArea())
            .navigationTitle(root.title)
            .navigationBarBackButtonHidden(true)
            .toolbar {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    VideoLink(route: VideoRoute.wall(VideoWallQuery(
                        library: nil, parent: nil, kind: nil, genre: nil, search: "", sort: .title,
                        unplayedOnly: false, cursor: nil, source: model.source
                    ), title: String(localized: .Video1.videoSearch))) {
                        Image(systemName: "magnifyingglass")
                    }
                    .accessibilityLabel(String(localized: .Video1.videoSearch))
                    Menu {
                        Button(String(localized: .Video0.videoWebVersion), systemImage: "globe", action: root.openWeb)
                        Button(TabText.addToMyApps, systemImage: "plus.square.on.square", action: root.addToMyApps)
                        Button(TabText.details, systemImage: "info.circle", action: root.details)
                    } label: {
                        Label(TabText.more, systemImage: "ellipsis")
                    }
                }
            }
            .task { if case .loading = load { await refresh() } }
            .refreshable { await refresh() }
    }

    @ViewBuilder private var content: some View {
        switch load {
        case .loading:
            LoadingView().frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .failed(problem):
            ProblemView(problem: problem) { Task { await reload() } }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .loaded(home):
            loaded(home)
        }
    }

    private func loaded(_ home: VideoHome) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let failure = home.partialFailure {
                    ProblemRow(problem: failure.videoProblem) { Task { await refresh() } }
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                }
                if !home.libraries.isEmpty { libraries(home.libraries) }
                ForEach(home.shelves, id: \.kind) { shelf($0) }
                if home.shelves.isEmpty && home.libraries.isEmpty {
                    Text(.Video2.videoEmpty)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(maxWidth: .infinity)
                        .padding(WontonTheme.spacing.s32)
                }
            }
            .padding(.vertical, WontonTheme.spacing.s16)
        }
    }

    private func libraries(_ libraries: [VideoLibrary]) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s8) {
                ForEach(libraries, id: \.id) { library in
                    VideoLink(route: VideoRoute.wall(.library(library, source: model.source), title: library.title)) {
                        Label(library.title, systemImage: library.kind.symbol)
                            .wontonText(WontonTheme.typography.supporting)
                            .fontWeight(.medium)
                            .foregroundStyle(Color(colors.textPrimary))
                            .padding(.horizontal, WontonTheme.spacing.s16)
                            .padding(.vertical, WontonTheme.spacing.s8)
                            .background(Color(colors.surfaceSubtle), in: Capsule())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }

    @ViewBuilder private func shelf(_ shelf: VideoShelf) -> some View {
        switch shelf.kind {
        case .resume, .nextUp:
            VideoShelfRow(title: shelf.kind.title, entries: shelf.entries) { entry in
                VideoLink(route: entry.route) { VideoLandscapeCard(entry: entry, width: 280) }
                    .buttonStyle(.plain)
            }
        case .latest:
            VideoShelfRow(title: shelf.kind.title, entries: shelf.entries) { entry in
                VideoLink(route: entry.route) { VideoPosterCard(entry: entry, width: 128) }
                    .buttonStyle(.plain)
            }
        }
    }

    private func reload() async {
        load = .loading
        await refresh()
    }

    private func refresh() async {
        switch await model.home() {
        case let .success(home): load = .loaded(home)
        case let .failure(error): load = .failed(error.videoProblem)
        }
    }
}

extension VideoShelfKind {
    var title: String {
        switch self {
        case .resume: String(localized: .Video1.videoShelfResume)
        case .nextUp: String(localized: .Video2.videoShelfNextUp)
        case .latest: String(localized: .Video1.videoShelfLatest)
        }
    }
}

extension VideoLibraryKind {
    var symbol: String {
        switch self {
        case .movies: "film"
        case .shows: "tv"
        case .mixed, .other: "square.stack"
        }
    }
}
