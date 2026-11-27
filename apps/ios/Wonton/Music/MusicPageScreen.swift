import SwiftUI
import WontonClient

/// 音乐库入口页右上角“更多”里的几项：和网页的胶囊菜单一样，网页版也从这里进。
struct MusicRootActions {
    let title: String
    let openWeb: () -> Void
    let addToMyApps: () -> Void
    let details: () -> Void
}

/// 一页音乐内容：模型在第一次出现时建（建就是订阅、开始取），加载、整页出错和部分失败都在这里，内容交给各页。
struct MusicPageScreen: View {
    let route: MusicRoute
    let context: MusicContext
    var root: MusicRootActions?
    @State private var model: MusicPageModel?

    var body: some View {
        Group {
            if let model {
                MusicPageBody(model: model, root: root)
            } else {
                Color.clear
            }
        }
        .environment(\.music, context)
        .onAppear {
            if model == nil {
                model = MusicPageModel(core: context.core, pluginId: route.pluginId, page: route.page)
            }
        }
    }
}

private struct MusicPageBody: View {
    let model: MusicPageModel
    let root: MusicRootActions?
    @State private var query = ""
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .background(Color(colors.canvas).ignoresSafeArea())
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(largeTitle ? .large : .inline)
            .navigationBarBackButtonHidden(root != nil)
            .toolbar { toolbar }
            .modifier(MusicSearchField(enabled: model.view.page == .search, query: $query))
            .onChange(of: query) { _, text in model.act(.query(text: text)) }
            .wontonDialog(problem: model.actionProblem, isPresented: Binding(
                get: { model.actionProblem != nil }, set: { if !$0 { model.actionProblem = nil } }
            ))
    }

    @ViewBuilder private var content: some View {
        switch model.view.state {
        case .loading:
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .failed(problem):
            ProblemView(problem: problem.problem) { model.act(.reload) }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .ready(content, hasMore, loadingMore, problem):
            MusicContentView(
                content: content,
                paging: MusicPaging(hasMore: hasMore, loadingMore: loadingMore) { model.act(.loadMore) },
                notice: problem.map(\.problem),
                reload: { model.act(.reload) },
                star: model.star
            )
        }
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        if let root {
            ToolbarItemGroup(placement: .topBarTrailing) {
                MusicOpenButton(
                    page: .search, label: String(localized: .Music1.musicSearch), systemImage: "magnifyingglass"
                )
                Menu {
                    Button(String(localized: .Music0.musicWebVersion), systemImage: "globe", action: root.openWeb)
                    Button(TabText.addToMyApps, systemImage: "plus.square.on.square", action: root.addToMyApps)
                    Button(TabText.details, systemImage: "info.circle", action: root.details)
                } label: {
                    Label(TabText.more, systemImage: "ellipsis")
                }
                .accessibilityIdentifier("musicMore")
            }
        }
        if case let .albums(sort) = model.view.page {
            ToolbarItem(placement: .topBarTrailing) { MusicSortMenu(sort: sort) }
        }
    }

    private var largeTitle: Bool {
        switch model.view.page {
        case .album, .artist, .playlist: false
        default: true
        }
    }

    private var title: String {
        switch model.view.page {
        case .home: root?.title ?? ""
        case .albums: String(localized: .Music1.musicAlbums)
        case .artists: String(localized: .Music1.musicArtists)
        case .playlists: String(localized: .Music0.musicPlaylists)
        case .songs: String(localized: .Music1.musicSongs)
        case .starred: String(localized: .Music1.musicStarred)
        case .genres: String(localized: .Music0.musicGenres)
        case let .genre(name): name
        case .search: String(localized: .Music1.musicSearch)
        case .album, .artist, .playlist: ""
        }
    }
}

/// 换专辑排序：推一页新的专辑页会多一层返回，所以原地换掉这一页。
private struct MusicSortMenu: View {
    let sort: AlbumSort
    @Environment(\.music) private var music

    var body: some View {
        Menu {
            ForEach(MusicSortMenu.sorts, id: \.self) { option in
                Button {
                    music?.replace(.albums(sort: option))
                } label: {
                    if option == sort {
                        Label(Self.name(option), systemImage: "checkmark")
                    } else {
                        Text(Self.name(option))
                    }
                }
            }
        } label: {
            Label(String(localized: .Music0.musicSort), systemImage: "arrow.up.arrow.down")
        }
    }

    static let sorts: [AlbumSort] = [.newest, .recent, .frequent, .name, .artist, .random]

    static func name(_ sort: AlbumSort) -> String {
        switch sort {
        case .newest: String(localized: .Music0.musicSortNewest)
        case .recent: String(localized: .Music0.musicSortRecent)
        case .frequent: String(localized: .Music0.musicSortFrequent)
        case .name: String(localized: .Music0.musicSortName)
        case .artist: String(localized: .Music1.musicSortArtist)
        case .random: String(localized: .Music1.musicSortRandom)
        }
    }
}

private struct MusicOpenButton: View {
    let page: MusicPage
    let label: String
    let systemImage: String
    @Environment(\.music) private var music

    var body: some View {
        Button { music?.open(page) } label: { Label(label, systemImage: systemImage) }
    }
}

/// 搜索页的搜索框：只有搜索页有，一直露着。
private struct MusicSearchField: ViewModifier {
    let enabled: Bool
    @Binding var query: String

    func body(content: Content) -> some View {
        if enabled {
            content.searchable(
                text: $query,
                placement: .navigationBarDrawer(displayMode: .always),
                prompt: String(localized: .Music1.musicSearchPrompt)
            )
        } else {
            content
        }
    }
}

/// 翻页：滚到最后一项时要下一页。
struct MusicPaging {
    let hasMore: Bool
    let loadingMore: Bool
    let loadMore: () -> Void

    func reached() {
        if hasMore, !loadingMore { loadMore() }
    }
}
