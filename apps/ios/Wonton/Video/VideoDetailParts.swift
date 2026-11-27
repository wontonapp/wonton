import SwiftUI
import WontonClient

/// 详情顶上的大图：背景图往下渐隐到底色，片名（有台标图用台标图）压在下沿。
struct VideoHero: View {
    let entry: VideoEntry
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            VideoArtwork(url: entry.backdrop ?? entry.landscape ?? entry.poster)
                .frame(maxWidth: .infinity)
                .frame(height: 300)
                .clipped()
                .overlay {
                    LinearGradient(
                        stops: [
                            .init(color: Color(colors.canvas).opacity(0), location: 0.35),
                            .init(color: Color(colors.canvas), location: 1)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                }
            title
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.bottom, WontonTheme.spacing.s4)
        }
    }

    @ViewBuilder private var title: some View {
        if let logo = entry.logo {
            VideoArtwork(url: logo) { titleText }
                .scaledToFit()
                .frame(maxWidth: 240, maxHeight: 80, alignment: .bottomLeading)
                .background(Color.clear)
        } else {
            titleText
        }
    }

    private var titleText: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            if entry.kind == .episode || entry.kind == .season, let series = entry.seriesTitle {
                Text(series)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            Text(entry.title)
                .wontonText(WontonTheme.typography.screenTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(2)
        }
    }
}

/// 片名下面一行：年份、分级、评分、片长、集号，再一行清晰度和音频的小标。
struct VideoFacts: View {
    let detail: VideoDetail
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(line)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            if !badges.isEmpty {
                HStack(spacing: WontonTheme.spacing.s4) {
                    ForEach(badges, id: \.self) { badge in
                        Text(badge)
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(Color(colors.textSecondary))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 2)
                            .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color(colors.separator)))
                    }
                }
            }
        }
    }

    private var line: String {
        let entry = detail.entry
        var parts: [String] = []
        if let label = VideoFormat.episodeLabel(entry) { parts.append(label) }
        if let year = entry.year { parts.append(String(year)) }
        if let rating = entry.officialRating { parts.append(rating) }
        if let score = entry.communityRating {
            parts.append("★ " + score.formatted(.number.precision(.fractionLength(1))))
        }
        if let duration = entry.durationMs, entry.kind != .series { parts.append(VideoFormat.duration(duration)) }
        if let count = entry.childCount, entry.kind == .series || entry.kind == .season {
            let label: LocalizedStringResource = entry.kind == .series
                ? .Video2.videoSeasons(Int(count))
                : .Video0.videoEpisodes(Int(count))
            parts.append(String(localized: label))
        }
        if !detail.genres.isEmpty { parts.append(detail.genres.prefix(3).joined(separator: "/")) }
        return parts.joined(separator: " · ")
    }

    /// 4K、HDR、编码、声道：从源的规格里来，没有就不写。
    private var badges: [String] {
        guard let media = detail.media else { return [] }
        var badges: [String] = []
        if let width = media.width {
            switch width {
            case 3200...: badges.append("4K")
            case 1800...: badges.append("1080p")
            case 1200...: badges.append("720p")
            default: badges.append("SD")
            }
        }
        if media.hdr { badges.append("HDR") }
        if let codec = media.videoCodec { badges.append(codec.uppercased()) }
        if let channels = media.audioChannels {
            badges.append(channels > 2 ? "\(channels - 1).1" : "\(channels).0")
        }
        return badges
    }
}

/// 音轨和字幕：选好的号在起播时交给核心，由提供方挑流或转码时烧进去。
struct VideoTrackMenus: View {
    let detail: VideoDetail
    @Binding var audio: UInt16?
    @Binding var subtitle: UInt16?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            if detail.audio.count > 1 {
                Menu {
                    Picker(String(localized: .Video0.videoAudio), selection: $audio) {
                        ForEach(detail.audio, id: \.index) { Text($0.name).tag(UInt16?.some($0.index)) }
                    }
                } label: {
                    chip("speaker.wave.2", selected: detail.audio.first { $0.index == audio }?.name)
                }
            }
            if !detail.subtitles.isEmpty {
                Menu {
                    Picker(String(localized: .Video0.videoSubtitles), selection: $subtitle) {
                        Text(.Video2.videoSubtitlesOff).tag(UInt16?.none)
                        ForEach(detail.subtitles, id: \.index) { Text($0.name).tag(UInt16?.some($0.index)) }
                    }
                } label: {
                    chip(
                        "captions.bubble",
                        selected: detail.subtitles.first { $0.index == subtitle }?.name
                            ?? String(localized: .Video2.videoSubtitlesOff)
                    )
                }
            }
            Spacer(minLength: 0)
        }
    }

    private func chip(_ symbol: String, selected: String?) -> some View {
        Label(selected ?? "", systemImage: symbol)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textPrimary))
            .lineLimit(1)
            .padding(.horizontal, WontonTheme.spacing.s12)
            .padding(.vertical, WontonTheme.spacing.s8)
            .background(Color(colors.surfaceSubtle), in: Capsule())
    }
}

extension MediaTrack {
    /// 菜单里显示的名字：提供方的标签，没有就用系统的语言名。
    var name: String {
        if let label, !label.isEmpty { return label }
        return Locale.current.localizedString(forIdentifier: language) ?? language
    }
}

/// 下载中：进度条，和一句“下载时保持 App 打开”。App 退到后台久了系统会停掉下载，回来接着下。
struct VideoOfflineRow: View {
    let state: OfflineState?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if case let .downloading(done, total) = state {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                if let total, total > 0 {
                    ProgressView(value: Double(done), total: Double(total))
                } else {
                    ProgressView(value: nil as Double?)
                        .progressViewStyle(.linear)
                }
                HStack {
                    Text(.Video2.videoDownloading)
                    Spacer(minLength: 0)
                    Text(sizes(done: done, total: total))
                }
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                Text(.Video0.videoKeepAppOpen)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
    }

    private func sizes(done: UInt64, total: UInt64?) -> String {
        let format = ByteCountFormatStyle(style: .file)
        guard let total else { return Int64(done).formatted(format) }
        return "\(Int64(done).formatted(format)) / \(Int64(total).formatted(format))"
    }
}

/// 一季的集：左边剧照（带进度），右边集号、名字和片长。
struct VideoEpisodeList: View {
    let entries: [VideoEntry]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        LazyVStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            ForEach(entries, id: \.id) { entry in
                VideoLink(route: entry.route) { row(entry) }
                    .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }

    private func row(_ entry: VideoEntry) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            VideoArtwork(url: entry.landscape ?? entry.backdrop)
                .frame(width: 144, height: 81)
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                .overlay(alignment: .bottom) {
                    if let progress = entry.progress { VideoProgressBar(progress: progress).padding(6) }
                }
                .overlay(alignment: .topTrailing) {
                    if entry.played {
                        Image(systemName: "checkmark.circle.fill")
                            .symbolRenderingMode(.palette)
                            .foregroundStyle(.white, .black.opacity(0.55))
                            .padding(4)
                    }
                }
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(entry.episode.map { "\($0). \(entry.title)" } ?? entry.title)
                    .wontonText(WontonTheme.typography.supporting)
                    .fontWeight(.medium)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(2)
                if let line = VideoFormat.remaining(entry) ?? entry.durationMs.map(VideoFormat.duration) {
                    Text(line)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
    }
}

/// 演职员：圆头像一排。
struct VideoCast: View {
    let people: [VideoPerson]
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Text(.Video1.videoCast)
                .wontonText(WontonTheme.typography.sectionTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .accessibilityAddTraits(.isHeader)
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                    ForEach(Array(people.enumerated()), id: \.offset) { _, person in card(person) }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
        }
    }

    private func card(_ person: VideoPerson) -> some View {
        VStack(spacing: WontonTheme.spacing.s4) {
            VideoArtwork(url: person.photo) {
                Image(systemName: "person.fill").foregroundStyle(Color(colors.textSecondary))
            }
            .frame(width: 72, height: 72)
            .clipShape(Circle())
            Text(person.name)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
            Text(person.character ?? person.role.name)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                .lineLimit(1)
        }
        .frame(width: 84)
    }
}

extension VideoRole {
    var name: String {
        switch self {
        case .actor: String(localized: .Video1.videoRoleActor)
        case .director: String(localized: .Video1.videoRoleDirector)
        case .writer: String(localized: .Video1.videoRoleWriter)
        case .producer: String(localized: .Video2.videoRoleProducer)
        case .other: ""
        }
    }
}
