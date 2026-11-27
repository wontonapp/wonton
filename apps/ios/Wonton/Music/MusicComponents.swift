import SwiftUI
import WontonClient

/// 封面：取到之前是带音符的底色块，取不到也一直是它（插件没给封面、封面坏了）。
struct MusicCover: View {
    let cover: String?
    let side: CGFloat
    var round = false
    @Environment(\.music) private var music
    @Environment(\.displayScale) private var scale
    @Environment(\.wontonColors) private var colors
    @State private var image: UIImage?

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: round ? side / 2 : max(4, side * 0.06), style: .continuous)
        ZStack {
            Color(colors.surfaceSubtle)
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                WontonIcon.music.view(size: side * 0.36).foregroundStyle(Color(colors.textSecondary).opacity(0.5))
            }
        }
        .frame(width: side, height: side)
        .clipShape(shape)
        .overlay { shape.strokeBorder(Color(colors.separator).opacity(0.5), lineWidth: 0.5) }
        .task(id: cover) { await load() }
    }

    private func load() async {
        guard let cover, let music else { image = nil; return }
        let pixels = UInt32((side * scale).rounded(.up))
        image = await MusicCovers.shared.load(core: music.core, pluginId: music.pluginId, cover: cover, pixels: pixels)
    }
}

/// 封面墙和横排里的一张专辑：点开是专辑页，长按收藏。
struct MusicAlbumTile: View {
    let album: AlbumTile
    let side: CGFloat
    /// 歌手页上每张都是同一个歌手，第二行改写年份。
    var showsYear = false
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button { music?.open(.album(id: album.id)) } label: {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                MusicCover(cover: album.cover, side: side)
                Text(album.title)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                if let second = showsYear ? album.year.map(String.init) : album.artist {
                    Text(second)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
            }
            .frame(width: side, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { MusicStarButton(starred: album.starred) { star(.album(album.id), $0) } }
    }
}

/// 收藏和取消收藏，长按菜单里的一项。
struct MusicStarButton: View {
    let starred: Bool
    let set: (Bool) -> Void

    var body: some View {
        Button { set(!starred) } label: {
            Label(
                starred ? String(localized: .Music1.musicUnstar) : String(localized: .Music0.musicStar),
                systemImage: starred ? "star.slash" : "star"
            )
        }
    }
}

/// 一首歌：封面或曲序、歌名、署名、时长。播放接上之前不能点。
struct MusicTrackRow: View {
    let track: TrackRow
    /// 专辑页里不重复封面和专辑名，显示曲序。
    var inAlbum = false
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            if inAlbum {
                Text(track.track.map(String.init) ?? "")
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .monospacedDigit()
                    .frame(width: 24, alignment: .trailing)
            } else {
                MusicCover(cover: track.cover, side: WontonTheme.size.fileThumbnail)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(track.title)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                if let subtitle {
                    Text(subtitle)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
            }
            Spacer(minLength: WontonTheme.spacing.s8)
            if track.starred {
                WontonIcon.starFill.view(size: WontonTheme.size.iconTiny).foregroundStyle(Color(colors.textSecondary))
            }
            Text(MusicFormat.clock(track.durationMs))
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .monospacedDigit()
        }
        .padding(.vertical, WontonTheme.spacing.s4)
        .contentShape(Rectangle())
        .contextMenu { MusicStarButton(starred: track.starred) { star(.song(track.id), $0) } }
    }

    private var subtitle: String? {
        if inAlbum { return track.artist }
        return [track.artist, track.album].compactMap(\.self).joined(separator: " · ").nilIfEmpty
    }
}

/// 一位歌手：圆头像、名字、几张专辑。
struct MusicArtistRow: View {
    let artist: ArtistRow
    let star: (MusicTarget, Bool) -> Void
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button { music?.open(.artist(id: artist.id)) } label: {
            HStack(spacing: WontonTheme.spacing.s12) {
                MusicCover(cover: artist.cover, side: WontonTheme.size.avatar, round: true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(artist.name)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                    Text(String(localized: .Music0.musicAlbumCount(Int(artist.albumCount))))
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { MusicStarButton(starred: artist.starred) { star(.artist(artist.id), $0) } }
    }
}

/// 一张歌单：封面、名字、谁的（别人的才写）和几首。
struct MusicPlaylistRow: View {
    let playlist: PlaylistRow
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button { music?.open(.playlist(id: playlist.id)) } label: {
            HStack(spacing: WontonTheme.spacing.s12) {
                MusicCover(cover: playlist.cover, side: 56)
                VStack(alignment: .leading, spacing: 2) {
                    Text(playlist.name)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                    Text(MusicFormat.playlistLine(playlist))
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// 首页和收藏里的一段：标题，可选的“全部”入口。
struct MusicSectionHeader: View {
    let title: String
    var all: MusicPage?
    @Environment(\.music) private var music
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
            Spacer(minLength: 0)
            if let all {
                Button(String(localized: .Music1.musicSeeAll)) { music?.open(all) }
                    .wontonText(WontonTheme.typography.supporting)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }
}

enum MusicFormat {
    /// 歌的时长：`3:07`，一小时以上 `1:02:07`。
    static func clock(_ milliseconds: UInt64) -> String {
        let seconds = Int(milliseconds / 1000)
        let (hours, minutes, rest) = (seconds / 3600, seconds / 60 % 60, seconds % 60)
        return hours > 0
            ? String(format: "%d:%02d:%02d", hours, minutes, rest)
            : String(format: "%d:%02d", minutes, rest)
    }

    /// 专辑和歌单底下的一行：几首、多少分钟。
    static func summary(songs: UInt32, milliseconds: UInt64) -> String {
        let minutes = Int((milliseconds + 59_999) / 60_000)
        let count = String(localized: .Music1.musicSongCount(Int(songs)))
        let length = String(localized: .Music0.musicMinutes(minutes))
        return String(localized: .Music0.musicSummary(count, length))
    }

    static func playlistLine(_ playlist: PlaylistRow) -> String {
        let count = String(localized: .Music1.musicSongCount(Int(playlist.songCount)))
        guard let owner = playlist.owner else { return count }
        return "\(owner) · \(count)"
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
