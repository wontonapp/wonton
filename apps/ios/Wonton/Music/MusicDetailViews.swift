import SwiftUI
import WontonClient

/// 专辑、歌手、歌单页的头：大封面、标题、副标题、一行说明，收藏按钮在标题旁。
private struct MusicHeader: View {
    let cover: String?
    var round = false
    let title: String
    var subtitle: String?
    var openSubtitle: (() -> Void)?
    var caption: String?
    var starred: Bool?
    var setStar: ((Bool) -> Void)?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            MusicCover(cover: cover, side: round ? 180 : 240, round: round)
                .shadow(color: .black.opacity(0.18), radius: 16, y: 8)
            VStack(spacing: WontonTheme.spacing.s4) {
                Text(title)
                    .wontonText(WontonTheme.typography.sectionTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                if let subtitle {
                    Button { openSubtitle?() } label: {
                        Text(subtitle)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(openSubtitle == nil ? Color(colors.textSecondary) : Color(colors.primary))
                    }
                    .buttonStyle(.plain)
                    .disabled(openSubtitle == nil)
                }
                if let caption {
                    Text(caption)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .multilineTextAlignment(.center)
            if let starred, let setStar {
                Button { setStar(!starred) } label: {
                    Label(
                        starred ? String(localized: .Music1.musicStarredLabel) : String(localized: .Music0.musicStar),
                        systemImage: starred ? "star.fill" : "star"
                    )
                    .wontonText(WontonTheme.typography.supporting)
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.capsule)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.top, WontonTheme.spacing.s8)
    }
}

/// 专辑页：曲目按碟分段，只有一碟时不写碟号；最底下一行几首、多少分钟。
struct MusicAlbumView: View {
    let detail: AlbumDetail
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        List {
            if let notice { ProblemRow(problem: notice, retry: reload).listRowSeparator(.hidden) }
            MusicHeader(
                cover: detail.album.cover,
                title: detail.album.title,
                subtitle: detail.album.artist,
                openSubtitle: detail.album.artistId.map { id in { music?.open(.artist(id: id)) } },
                caption: [detail.genre, detail.album.year.map(String.init)].compactMap(\.self).joined(separator: " · ").nilIfEmpty,
                starred: detail.album.starred,
                setStar: { star(.album(detail.album.id), $0) }
            )
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets())
            .padding(.bottom, WontonTheme.spacing.s8)
            // 碟名是普通的一行，不用 Section 的表头：plain 列表的表头会钉在顶上、压住滚过去的曲目。
            ForEach(Array(detail.discs.enumerated()), id: \.offset) { _, disc in
                if let name = discName(disc) {
                    Text(name)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .listRowSeparator(.hidden)
                        .padding(.top, WontonTheme.spacing.s8)
                }
                ForEach(disc.tracks, id: \.id) { MusicTrackRow(track: $0, inAlbum: true, star: star) }
            }
            Text(MusicFormat.summary(songs: detail.songCount, milliseconds: detail.durationMs))
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                .listRowSeparator(.hidden)
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .refreshable { reload() }
    }

    private func discName(_ disc: DiscSection) -> String? {
        guard let number = disc.disc else { return nil }
        let label = String(localized: .Music0.musicDisc(Int(number)))
        guard let title = disc.title else { return label }
        return "\(label) · \(title)"
    }
}

/// 歌手页：头像、名字，下面是他的专辑墙。
struct MusicArtistView: View {
    let detail: ArtistDetail
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                if let notice { ProblemRow(problem: notice, retry: reload).padding(.horizontal) }
                MusicHeader(
                    cover: detail.artist.cover,
                    round: true,
                    title: detail.artist.name,
                    caption: String(localized: .Music0.musicAlbumCount(Int(detail.artist.albumCount))),
                    starred: detail.artist.starred,
                    setStar: { star(.artist(detail.artist.id), $0) }
                )
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 150), spacing: WontonTheme.spacing.s16, alignment: .top)],
                    spacing: WontonTheme.spacing.s24
                ) {
                    ForEach(detail.albums, id: \.id) { album in
                        MusicAlbumTile(album: album, side: 160, showsYear: true, star: star)
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
            .padding(.bottom, WontonTheme.spacing.s24)
        }
        .refreshable { reload() }
    }
}

/// 歌单页：封面、名字、谁的、说明，下面是歌。
struct MusicPlaylistView: View {
    let detail: PlaylistDetail
    let notice: Problem?
    let reload: () -> Void
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        List {
            if let notice { ProblemRow(problem: notice, retry: reload).listRowSeparator(.hidden) }
            MusicHeader(
                cover: detail.playlist.cover,
                title: detail.playlist.name,
                subtitle: detail.playlist.owner,
                caption: detail.comment
            )
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets())
            .padding(.bottom, WontonTheme.spacing.s8)
            // 同一首歌能在歌单里出现两次：按位置认行。
            ForEach(Array(detail.tracks.enumerated()), id: \.offset) { MusicTrackRow(track: $0.element, star: star) }
            Text(MusicFormat.summary(songs: detail.playlist.songCount, milliseconds: detail.playlist.durationMs))
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                .listRowSeparator(.hidden)
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .refreshable { reload() }
    }
}
