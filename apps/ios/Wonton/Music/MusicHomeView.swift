import SwiftUI
import WontonClient

/// 音乐库的首页：上面是几类入口，下面是最近添加、最近播放、常听和歌单几排。空的排不显示。
struct MusicHomeView: View {
    let home: MusicHome
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let notice { ProblemRow(problem: notice, retry: reload).padding(.horizontal) }
                MusicCategories()
                if !home.recentlyAdded.isEmpty {
                    MusicAlbumShelf(
                        title: String(localized: .Music1.musicRecentlyAdded),
                        albums: home.recentlyAdded,
                        all: .albums(sort: .newest),
                        star: star
                    )
                }
                if !home.recentlyPlayed.isEmpty {
                    MusicAlbumShelf(
                        title: String(localized: .Music0.musicRecentlyPlayed),
                        albums: home.recentlyPlayed,
                        all: .albums(sort: .recent),
                        star: star
                    )
                }
                if !home.mostPlayed.isEmpty {
                    MusicAlbumShelf(
                        title: String(localized: .Music0.musicMostPlayed),
                        albums: home.mostPlayed,
                        all: .albums(sort: .frequent),
                        star: star
                    )
                }
                if !home.playlists.isEmpty { MusicPlaylistShelf(playlists: home.playlists) }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .refreshable { reload() }
    }
}

/// 几类入口：一行一类，和系统“资料库”的样子一样。
private struct MusicCategories: View {
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    private var entries: [(String, WontonIcon, MusicPage)] {
        [
            (String(localized: .Music0.musicPlaylists), .list, .playlists),
            (String(localized: .Music1.musicArtists), .person, .artists),
            (String(localized: .Music1.musicAlbums), .imageFill, .albums(sort: .name)),
            (String(localized: .Music1.musicSongs), .music, .songs),
            (String(localized: .Music1.musicStarred), .star, .starred),
            (String(localized: .Music0.musicGenres), .folder, .genres),
        ]
    }

    var body: some View {
        VStack(spacing: 0) {
            ForEach(entries, id: \.0) { title, icon, page in
                Button { music?.open(page) } label: {
                    HStack(spacing: WontonTheme.spacing.s16) {
                        icon.view(size: WontonTheme.size.icon).foregroundStyle(Color(colors.textPrimary))
                        Text(title)
                            .wontonText(WontonTheme.typography.body)
                            .foregroundStyle(Color(colors.textPrimary))
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                    .frame(minHeight: WontonTheme.size.target)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                if title != entries.last?.0 {
                    Divider().padding(.leading, WontonTheme.size.icon + WontonTheme.spacing.s16)
                }
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }
}

/// 一排横着滑的专辑。
struct MusicAlbumShelf: View {
    let title: String
    let albums: [AlbumTile]
    var all: MusicPage?
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            MusicSectionHeader(title: title, all: all)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                    ForEach(albums, id: \.id) { MusicAlbumTile(album: $0, side: 150, star: star) }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
        }
    }
}

/// 一排横着滑的歌单。
private struct MusicPlaylistShelf: View {
    let playlists: [PlaylistRow]
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            MusicSectionHeader(title: String(localized: .Music0.musicPlaylists), all: .playlists)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                    ForEach(playlists, id: \.id) { playlist in
                        Button { music?.open(.playlist(id: playlist.id)) } label: {
                            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                                MusicCover(cover: playlist.cover, side: 150)
                                Text(playlist.name)
                                    .wontonText(WontonTheme.typography.supporting)
                                    .foregroundStyle(Color(colors.textPrimary))
                                    .lineLimit(1)
                                Text(MusicFormat.playlistLine(playlist))
                                    .wontonText(WontonTheme.typography.supporting)
                                    .foregroundStyle(Color(colors.textSecondary))
                                    .lineLimit(1)
                            }
                            .frame(width: 150, alignment: .leading)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
        }
    }
}

/// 一排横着滑的歌手头像。
struct MusicArtistShelf: View {
    let artists: [ArtistRow]
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            MusicSectionHeader(title: String(localized: .Music1.musicArtists))
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                    ForEach(artists, id: \.id) { artist in
                        Button { music?.open(.artist(id: artist.id)) } label: {
                            VStack(spacing: WontonTheme.spacing.s8) {
                                MusicCover(cover: artist.cover, side: 110, round: true)
                                Text(artist.name)
                                    .wontonText(WontonTheme.typography.supporting)
                                    .foregroundStyle(Color(colors.textPrimary))
                                    .lineLimit(1)
                            }
                            .frame(width: 110)
                        }
                        .buttonStyle(.plain)
                        .contextMenu { MusicStarButton(starred: artist.starred) { star(.artist(artist.id), $0) } }
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
        }
    }
}

/// 滚动页里的一段歌：标题加一列行。
struct MusicTrackStack: View {
    let title: String
    let tracks: [TrackRow]
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            MusicSectionHeader(title: title)
            LazyVStack(spacing: 0) {
                ForEach(tracks, id: \.id) { track in
                    MusicTrackRow(track: track, star: star)
                    if track.id != tracks.last?.id { Divider().padding(.leading, 44 + WontonTheme.spacing.s12) }
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }
}
