import SwiftUI
import WontonClient

/// 详情：大背景图和片名，播放（接着看）、从头看、看过、收藏、下载，音轨字幕，简介，季和集，演职员。
struct VideoDetailView: View {
    let model: VideoModel
    let id: String
    let source: String?
    @State private var load: VideoLoad<VideoDetail> = .loading
    @State private var audio: UInt16?
    @State private var subtitle: UInt16?
    @State private var offline: OfflineState?
    @State private var season: VideoEntry?
    @State private var episodes: VideoLoad<[VideoEntry]> = .loading
    @State private var overviewExpanded = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        content
            .navigationBarTitleDisplayMode(.inline)
            .task { await refresh() }
            .task(id: season?.id) { await loadEpisodes() }
            .task(id: isDownloading) { await watchDownload() }
    }

    @ViewBuilder private var content: some View {
        switch load {
        case .loading:
            LoadingView().frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .failed(problem):
            ProblemView(problem: problem) { Task { load = .loading; await refresh() } }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .loaded(detail):
            loaded(detail)
        }
    }

    private func loaded(_ detail: VideoDetail) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                VideoHero(entry: detail.entry)
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                    VideoFacts(detail: detail)
                    actions(detail)
                    if let target = playTarget(detail), target.id != detail.entry.id {
                        Text(VideoFormat.episodeLabel(target).map { "\($0) · \(target.title)" } ?? target.title)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                    if detail.entry.item != nil { VideoOfflineRow(state: offline) }
                    if detail.audio.count > 1 || !detail.subtitles.isEmpty {
                        VideoTrackMenus(detail: detail, audio: $audio, subtitle: $subtitle)
                    }
                    overview(detail)
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                if detail.entry.kind == .series, !detail.children.isEmpty {
                    seasons(detail.children)
                } else if detail.entry.kind == .season, !detail.children.isEmpty {
                    VideoEpisodeList(entries: detail.children)
                }
                if !detail.people.isEmpty { VideoCast(people: detail.people) }
            }
            .padding(.bottom, WontonTheme.spacing.s32)
        }
        .ignoresSafeArea(edges: .top)
        .background(Color(colors.canvas))
    }

    // MARK: 按钮

    /// 剧放“下一集”，季放第一集没看完的，其余放自己。
    private func playTarget(_ detail: VideoDetail) -> VideoEntry? {
        switch detail.entry.kind {
        case .series: detail.nextUp
        case .season: detail.nextUp ?? detail.children.first { !$0.played && $0.item != nil }
        default: detail.entry.item == nil ? nil : detail.entry
        }
    }

    private func actions(_ detail: VideoDetail) -> some View {
        let target = playTarget(detail)
        let resume = target?.positionMs.flatMap { $0 > 0 ? $0 : nil }
        return HStack(spacing: WontonTheme.spacing.s8) {
            if let target, let item = target.item {
                Button {
                    Task {
                        await model.play(item, title: target.title, from: resume ?? 0, audio: audio, subtitle: subtitle)
                    }
                } label: {
                    Label(
                        resume == nil ? String(localized: .Video2.videoPlay) : String(localized: .Video1.videoResume),
                        systemImage: "play.fill"
                    )
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                if resume != nil {
                    Button {
                        Task { await model.play(item, title: target.title, from: 0, audio: audio, subtitle: subtitle) }
                    } label: {
                        Image(systemName: "arrow.counterclockwise")
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                    .accessibilityLabel(String(localized: .Video2.videoRestart))
                }
            }
            iconButton(
                detail.entry.played ? "checkmark.circle.fill" : "checkmark.circle",
                label: detail.entry.played ? .Video2.videoMarkUnplayed : .Video0.videoMarkPlayed
            ) {
                if await model.setPlayed(detail.entry, !detail.entry.played) { await refresh() }
            }
            iconButton(
                detail.entry.favorite ? "heart.fill" : "heart",
                label: detail.entry.favorite ? .Video1.videoUnfavorite : .Video2.videoFavorite
            ) {
                if await model.setFavorite(detail.entry, !detail.entry.favorite) { await refresh() }
            }
            if let item = detail.entry.item { downloadButton(item) }
        }
    }

    private func iconButton(
        _ symbol: String,
        label: LocalizedStringResource,
        action: @escaping () async -> Void
    ) -> some View {
        Button { Task { await action() } } label: { Image(systemName: symbol) }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
            .accessibilityLabel(String(localized: label))
    }

    @ViewBuilder private func downloadButton(_ item: MediaItemRef) -> some View {
        switch offline {
        case .none:
            iconButton("arrow.down.circle", label: .Video0.videoDownload) {
                await model.download(item)
                offline = model.offline(item.id)
            }
        case .downloading:
            EmptyView()
        case .ready:
            Menu {
                Button(role: .destructive) {
                    model.removeOffline(item.id)
                    offline = model.offline(item.id)
                } label: {
                    Label(String(localized: .Video0.videoRemoveDownload), systemImage: "trash")
                }
            } label: {
                Image(systemName: "arrow.down.circle.fill")
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
            .accessibilityLabel(String(localized: .Video2.videoDownloaded))
        }
    }

    // MARK: 简介、季

    @ViewBuilder private func overview(_ detail: VideoDetail) -> some View {
        if let tagline = detail.tagline, !tagline.isEmpty {
            Text(tagline)
                .wontonText(WontonTheme.typography.body)
                .fontWeight(.semibold)
                .foregroundStyle(Color(colors.textPrimary))
        }
        if let overview = detail.overview, !overview.isEmpty {
            Text(overview)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textSecondary))
                .lineLimit(overviewExpanded ? nil : 4)
                .onTapGesture { withAnimation { overviewExpanded.toggle() } }
        }
    }

    private func seasons(_ seasons: [VideoEntry]) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    ForEach(seasons, id: \.id) { entry in
                        let selected = entry.id == season?.id
                        Button { season = entry } label: {
                            Text(entry.title)
                                .wontonText(WontonTheme.typography.supporting)
                                .fontWeight(selected ? .semibold : .regular)
                                .foregroundStyle(Color(selected ? colors.canvas : colors.textPrimary))
                                .padding(.horizontal, WontonTheme.spacing.s16)
                                .padding(.vertical, WontonTheme.spacing.s8)
                                .background(Color(selected ? colors.textPrimary : colors.surfaceSubtle), in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
            }
            switch episodes {
            case .loading:
                ProgressView().frame(maxWidth: .infinity).padding(WontonTheme.spacing.s16)
            case let .failed(problem):
                ProblemRow(problem: problem) { Task { await loadEpisodes() } }
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
            case let .loaded(entries):
                VideoEpisodeList(entries: entries)
            }
        }
    }

    // MARK: 取数据

    private func refresh() async {
        switch await model.detail(id, source: source) {
        case let .success(detail):
            load = .loaded(detail)
            offline = detail.entry.item.flatMap { model.offline($0.id) } ?? detail.offline
            if audio == nil { audio = detail.audio.first(where: \.selected)?.index }
            if subtitle == nil { subtitle = detail.subtitles.first(where: \.selected)?.index }
            if detail.entry.kind == .series, season == nil {
                // 默认选“下一集”所在的季，没有就第一季。
                let next = detail.nextUp?.season
                season = detail.children.first { $0.season == next && next != nil } ?? detail.children.first
            }
        case let .failure(error):
            if case .loaded = load {
                model.actionProblem = error.videoProblem
            } else {
                load = .failed(error.videoProblem)
            }
        }
    }

    private func loadEpisodes() async {
        guard let season else { return }
        episodes = .loading
        switch await model.detail(season.id, source: season.source ?? source) {
        case let .success(detail): episodes = .loaded(detail.children)
        case let .failure(error): episodes = .failed(error.videoProblem)
        }
    }

    private var isDownloading: Bool {
        if case .downloading = offline { return true }
        return false
    }

    /// 下载进度在客户端核里，每秒看一次，直到写完或没了。
    private func watchDownload() async {
        guard isDownloading, case let .loaded(detail) = load, let item = detail.entry.item else { return }
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(1))
            let state = model.offline(item.id)
            if state != offline { offline = state }
            if case .downloading = state { continue }
            return
        }
    }
}
