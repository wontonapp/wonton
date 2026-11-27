import SwiftUI
import WontonClient

/// 影视页里的几种卡：竖版海报、横版剧照（带进度条）。点卡进详情。
enum VideoFormat {
    /// `S1 · E3`；只有集号时 `E3`。
    static func episodeLabel(_ entry: VideoEntry) -> String? {
        switch (entry.season, entry.episode) {
        case let (season?, episode?): "S\(season) · E\(episode)"
        case let (nil, episode?): "E\(episode)"
        default: nil
        }
    }

    /// 片长或剩下多久，按系统语言写（“1小时42分钟”）。
    static func duration(_ milliseconds: UInt64) -> String {
        let formatter = DateComponentsFormatter()
        formatter.allowedUnits = milliseconds >= 3_600_000 ? [.hour, .minute] : [.minute]
        formatter.unitsStyle = .abbreviated
        return formatter.string(from: TimeInterval(milliseconds / 1000)) ?? ""
    }

    static func remaining(_ entry: VideoEntry) -> String? {
        guard let duration = entry.durationMs, let position = entry.positionMs, position < duration else { return nil }
        return String(localized: .Video1.videoRemaining(self.duration(duration - position)))
    }

    /// 卡片下面那行小字：单集写剧名和集号，其余写年份。
    static func caption(_ entry: VideoEntry) -> String? {
        if entry.kind == .episode {
            return [entry.seriesTitle, episodeLabel(entry)].compactMap { $0 }.joined(separator: " · ")
        }
        return entry.year.map { String($0) }
    }
}

/// 竖版海报卡：2:3 的图，片名和年份在下面。看完的角上打勾，剧集角上是没看的集数。
struct VideoPosterCard: View {
    let entry: VideoEntry
    let width: CGFloat?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            VideoArtwork(url: entry.poster) { placeholderTitle }
                .aspectRatio(2 / 3, contentMode: .fit)
                .frame(width: width)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay(alignment: .topTrailing) { badge.padding(6) }
                .overlay(alignment: .bottom) { progressBar }
            Text(entry.title)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
            if let caption = VideoFormat.caption(entry) {
                Text(caption)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(1)
            }
        }
        .frame(width: width)
        .contentShape(Rectangle())
    }

    private var placeholderTitle: some View {
        Text(entry.title)
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(colors.textSecondary))
            .multilineTextAlignment(.center)
            .padding(WontonTheme.spacing.s8)
    }

    @ViewBuilder private var badge: some View {
        if entry.played {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 18, weight: .semibold))
                .symbolRenderingMode(.palette)
                .foregroundStyle(.white, .black.opacity(0.55))
        } else if let unplayed = entry.unplayedCount, unplayed > 0, entry.kind == .series || entry.kind == .season {
            Text("\(unplayed)")
                .font(.caption2.weight(.bold))
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .frame(minWidth: 20, minHeight: 20)
                .background(.black.opacity(0.55), in: Capsule())
        }
    }

    @ViewBuilder private var progressBar: some View {
        if let progress = entry.progress {
            VideoProgressBar(progress: progress).padding(6)
        }
    }
}

/// 横版卡：16:9 的剧照，片名、集号和还剩多久在下面。“继续观看”“下一集”用它。
struct VideoLandscapeCard: View {
    let entry: VideoEntry
    let width: CGFloat
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            VideoArtwork(url: entry.landscape ?? entry.backdrop)
                .frame(width: width, height: width * 9 / 16)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay(alignment: .bottom) {
                    if let progress = entry.progress { VideoProgressBar(progress: progress).padding(8) }
                }
            Text(entry.kind == .episode ? (entry.seriesTitle ?? entry.title) : entry.title)
                .wontonText(WontonTheme.typography.supporting)
                .fontWeight(.medium)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
            Text(line)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                .lineLimit(1)
        }
        .frame(width: width, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var line: String {
        var parts: [String] = []
        if entry.kind == .episode {
            if let label = VideoFormat.episodeLabel(entry) { parts.append(label) }
            parts.append(entry.title)
        }
        if let remaining = VideoFormat.remaining(entry), entry.progress != nil { parts.append(remaining) }
        return parts.joined(separator: " · ")
    }
}

/// 卡片底部的细进度条。
struct VideoProgressBar: View {
    let progress: Float

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(.black.opacity(0.45))
                Capsule().fill(.white).frame(width: geometry.size.width * CGFloat(min(max(progress, 0), 1)))
            }
        }
        .frame(height: 4)
    }
}

/// 一排横着滑的卡，标题右边可以有“全部”。
struct VideoShelfRow<Card: View>: View {
    let title: String
    let entries: [VideoEntry]
    var seeAll: (() -> Void)?
    @ViewBuilder let card: (VideoEntry) -> Card
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            HStack(alignment: .firstTextBaseline) {
                Text(title)
                    .wontonText(WontonTheme.typography.sectionTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 0)
                if let seeAll {
                    Button(String(localized: .Video2.videoSeeAll), action: seeAll)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
                    ForEach(entries, id: \.id) { card($0) }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
        }
    }
}
