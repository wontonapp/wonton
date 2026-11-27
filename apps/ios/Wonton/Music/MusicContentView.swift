import SwiftUI
import WontonClient

/// 按内容种类挑页面；部分失败（翻页、刷新没成）在最上面一条轻提示，内容留着。
struct MusicContentView: View {
    let content: MusicContent
    let paging: MusicPaging
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        switch content {
        case let .home(home):
            MusicHomeView(home: home, notice: notice, reload: reload, star: star)
        case let .albums(albums):
            MusicAlbumGrid(albums: albums, paging: paging, notice: notice, reload: reload, star: star)
        case let .artists(artists):
            MusicList(notice: notice, reload: reload) {
                ForEach(artists, id: \.id) { MusicArtistRow(artist: $0, star: star) }
            }
        case let .playlists(playlists):
            MusicList(notice: notice, reload: reload) {
                ForEach(playlists, id: \.id) { MusicPlaylistRow(playlist: $0) }
            }
        case let .songs(songs):
            MusicList(notice: notice, reload: reload) {
                ForEach(songs, id: \.id) { song in
                    MusicTrackRow(track: song, star: star)
                        .onAppear { if song.id == songs.last?.id { paging.reached() } }
                }
                if paging.loadingMore { MusicLoadingRow() }
            }
        case let .starred(starred):
            MusicStarredView(starred: starred, notice: notice, reload: reload, star: star)
        case let .genres(genres):
            MusicList(notice: notice, reload: reload) {
                ForEach(genres, id: \.name) { MusicGenreRow(genre: $0) }
            }
        case let .album(album):
            MusicAlbumView(detail: album, notice: notice, reload: reload, star: star)
        case let .artist(artist):
            MusicArtistView(detail: artist, notice: notice, reload: reload, star: star)
        case let .playlist(playlist):
            MusicPlaylistView(detail: playlist, notice: notice, reload: reload, star: star)
        case let .search(query, results):
            MusicSearchResults(query: query, results: results, notice: notice, star: star)
        }
    }
}

/// 一列行的页：系统的分组列表，下拉刷新。
struct MusicList<Rows: View>: View {
    let notice: Problem?
    let reload: () -> Void
    @ViewBuilder let rows: Rows

    var body: some View {
        List {
            if let notice { ProblemRow(problem: notice, retry: reload).listRowSeparator(.hidden) }
            rows
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .refreshable { reload() }
    }
}

struct MusicLoadingRow: View {
    var body: some View {
        ProgressView().frame(maxWidth: .infinity).listRowSeparator(.hidden)
    }
}

private struct MusicGenreRow: View {
    let genre: GenreRow
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button { music?.open(.genre(name: genre.name)) } label: {
            HStack {
                Text(genre.name)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                Spacer(minLength: WontonTheme.spacing.s8)
                Text(String(localized: .Music0.musicAlbumCount(Int(genre.albumCount))))
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            .padding(.vertical, WontonTheme.spacing.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// 专辑封面墙：两列，宽屏多列；滚到底要下一页。
struct MusicAlbumGrid: View {
    let albums: [AlbumTile]
    let paging: MusicPaging
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        GeometryReader { geometry in
            let inset = WontonTheme.spacing.pageInset
            let gap = WontonTheme.spacing.s16
            let columns = max(2, Int((geometry.size.width - inset * 2 + gap) / (180 + gap)))
            let side = (geometry.size.width - inset * 2 - gap * CGFloat(columns - 1)) / CGFloat(columns)
            ScrollView {
                if let notice { ProblemRow(problem: notice, retry: reload).padding(.horizontal, inset) }
                LazyVGrid(
                    columns: Array(repeating: GridItem(.fixed(side), spacing: gap, alignment: .top), count: columns),
                    spacing: WontonTheme.spacing.s24
                ) {
                    ForEach(albums, id: \.id) { album in
                        MusicAlbumTile(album: album, side: side, star: star)
                            .onAppear { if album.id == albums.last?.id { paging.reached() } }
                    }
                }
                .padding(.horizontal, inset)
                .padding(.vertical, WontonTheme.spacing.s8)
                if paging.loadingMore { ProgressView().padding() }
            }
            .refreshable { reload() }
        }
    }
}

/// 收藏：歌手、专辑、歌曲三段，空的段不显示。
private struct MusicStarredView: View {
    let starred: MusicStarred
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let notice { ProblemRow(problem: notice, retry: reload).padding(.horizontal) }
                if !starred.albums.isEmpty {
                    MusicAlbumShelf(title: String(localized: .Music1.musicAlbums), albums: starred.albums, star: star)
                }
                if !starred.artists.isEmpty {
                    MusicArtistShelf(artists: starred.artists, star: star)
                }
                if !starred.songs.isEmpty {
                    MusicTrackStack(title: String(localized: .Music1.musicSongs), tracks: starred.songs, star: star)
                }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .refreshable { reload() }
    }
}

/// 搜索结果：歌手、专辑、歌曲；有词没结果时说一句。
private struct MusicSearchResults: View {
    let query: String
    let results: MusicSearch
    let notice: Problem?
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let notice { ProblemRow(problem: notice).padding(.horizontal) }
                if !query.isEmpty, results.artists.isEmpty, results.albums.isEmpty, results.songs.isEmpty {
                    Text(String(localized: .Music0.musicNoResults(query)))
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(maxWidth: .infinity)
                        .padding(.top, WontonTheme.spacing.s48)
                }
                if !results.artists.isEmpty { MusicArtistShelf(artists: results.artists, star: star) }
                if !results.albums.isEmpty {
                    MusicAlbumShelf(title: String(localized: .Music1.musicAlbums), albums: results.albums, star: star)
                }
                if !results.songs.isEmpty {
                    MusicTrackStack(title: String(localized: .Music1.musicSongs), tracks: results.songs, star: star)
                }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .scrollDismissesKeyboard(.immediately)
    }
}
