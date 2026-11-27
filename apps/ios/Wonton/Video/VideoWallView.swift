import SwiftUI
import WontonClient

/// 海报墙：一个库、一个类型、文件夹或搜索结果。右上角是排序、只看没看过和类型；滑到底接着取下一页。
struct VideoWallView: View {
    let model: VideoModel
    let title: String
    @State private var query: VideoWallQuery
    @State private var load: VideoLoad<[VideoEntry]> = .loading
    @State private var cursor: String?
    @State private var loadingMore = false
    @State private var moreProblem: Problem?
    @State private var genres: [String] = []
    @State private var genresProblem: Problem?
    @State private var searchText = ""
    private let searching: Bool
    @Environment(\.wontonColors) private var colors

    init(model: VideoModel, query: VideoWallQuery, title: String) {
        self.model = model
        self.title = title
        _query = State(initialValue: query)
        searching = query.search != nil
    }

    var body: some View {
        content
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(searching ? .inline : .large)
            .toolbar { if !searching { ToolbarItem(placement: .topBarTrailing) { filterMenu } } }
            .modifier(SearchField(enabled: searching, text: $searchText))
            .task(id: query) { await reload() }
            .task(id: searchText) {
                guard searching else { return }
                // 打字停 0.3 秒再搜，别每个字都问一次服务器。
                try? await Task.sleep(for: .milliseconds(300))
                guard !Task.isCancelled else { return }
                query.search = searchText
            }
            .task { if !searching { await loadGenres() } }
    }

    @ViewBuilder private var content: some View {
        switch load {
        case .loading:
            if searching && query.search?.isEmpty != false {
                Color.clear
            } else {
                LoadingView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        case let .failed(problem):
            ProblemView(problem: problem) { Task { await reload() } }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .loaded(entries) where entries.isEmpty:
            Text(searching ? .Video2.videoNoResults : .Video2.videoEmpty)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .loaded(entries):
            grid(entries)
        }
    }

    private func grid(_ entries: [VideoEntry]) -> some View {
        ScrollView {
            LazyVGrid(
                columns: [
                    GridItem(.adaptive(minimum: 104, maximum: 160), spacing: WontonTheme.spacing.s12, alignment: .top)
                ],
                spacing: WontonTheme.spacing.s16
            ) {
                ForEach(entries, id: \.id) { entry in
                    VideoLink(route: entry.route) { VideoPosterCard(entry: entry, width: nil) }
                        .buttonStyle(.plain)
                        .onAppear { if entry.id == entries.last?.id { Task { await more() } } }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
            if loadingMore {
                ProgressView().padding(WontonTheme.spacing.s16)
            } else if let moreProblem {
                ProblemRow(problem: moreProblem) { Task { await more() } }
                    .padding(WontonTheme.spacing.pageInset)
            }
        }
    }

    private var filterMenu: some View {
        Menu {
            Picker(String(localized: .Video0.videoSort), selection: $query.sort) {
                Text(.Video2.videoSortTitle).tag(VideoSort.title)
                Text(.Video1.videoSortAdded).tag(VideoSort.added)
                Text(.Video0.videoSortReleased).tag(VideoSort.released)
                Text(.Video1.videoSortRating).tag(VideoSort.rating)
            }
            Toggle(String(localized: .Video2.videoUnplayedOnly), isOn: $query.unplayedOnly)
            if !genres.isEmpty {
                Picker(String(localized: .Video0.videoGenre), selection: $query.genre) {
                    Text(.Video2.videoGenreAll).tag(String?.none)
                    ForEach(genres, id: \.self) { Text($0).tag(String?.some($0)) }
                }
                .pickerStyle(.menu)
            } else if let genresProblem {
                // 类型没取到：说原因，点了再取。
                Section(String(localized: .Video0.videoGenre)) {
                    Button { Task { await loadGenres() } } label: {
                        Label(genresProblem.title, systemImage: "arrow.clockwise")
                    }
                }
            }
        } label: {
            Image(systemName: "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel(String(localized: .Video1.videoFilter))
    }

    private func reload() async {
        var first = query
        first.cursor = nil
        if searching && first.search?.isEmpty != false {
            load = .loading
            return
        }
        load = .loading
        moreProblem = nil
        switch await model.wall(first) {
        case let .success(wall):
            cursor = wall.cursor
            load = .loaded(wall.entries)
        case let .failure(error):
            load = .failed(error.videoProblem)
        }
    }

    private func loadGenres() async {
        switch await model.genres(library: query.library) {
        case let .success(list):
            genres = list
            genresProblem = nil
        case let .failure(error):
            genresProblem = error.videoProblem
        }
    }

    private func more() async {
        guard let cursor, !loadingMore, case let .loaded(entries) = load else { return }
        loadingMore = true
        moreProblem = nil
        var next = query
        next.cursor = cursor
        let result = await model.wall(next)
        loadingMore = false
        // 取的时候换了排序、筛选或搜索词：这一页属于旧的查询，丢掉。
        var current = query
        current.cursor = cursor
        guard current == next, self.cursor == cursor else { return }
        switch result {
        case let .success(wall):
            self.cursor = wall.cursor
            let seen = Set(entries.map(\.id))
            load = .loaded(entries + wall.entries.filter { !seen.contains($0.id) })
        case let .failure(error):
            moreProblem = error.videoProblem
        }
    }
}

/// 搜索页才有搜索框，一进来就聚焦。
private struct SearchField: ViewModifier {
    let enabled: Bool
    @Binding var text: String
    @State private var active = false

    func body(content: Content) -> some View {
        if enabled {
            content
                .searchable(
                    text: $text,
                    isPresented: $active,
                    placement: .navigationBarDrawer(displayMode: .always),
                    prompt: String(localized: .Video0.videoSearchPrompt)
                )
                .onAppear { active = true }
        } else {
            content
        }
    }
}
