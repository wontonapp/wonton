import SwiftUI
import WontonClient

/// 音乐页底部的迷你播放条：封面、歌名、播放暂停和下一首；点开是播放页。
struct MusicMiniPlayer: View {
    let player: MusicPlayer
    @State private var showsPlayer = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: 0) {
            if player.isActive, let item = player.view?.session.item {
                bar(item)
            }
        }
        // 播放页开着时由播放页弹。
        .wontonDialog(problem: player.problem, isPresented: Binding(
            get: { player.problem != nil && !showsPlayer }, set: { if !$0 { player.problem = nil } }
        ))
        .sheet(isPresented: $showsPlayer) {
            MusicNowPlayingView(player: player).environment(\.musicPlayer, player)
        }
        .environment(\.musicPlayer, player)
    }

    private func bar(_ item: MediaItemRef) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            MusicArtwork(item: item, side: 40)
            VStack(alignment: .leading, spacing: 0) {
                Text(item.title)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if let artist = item.artist {
                    Text(artist)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .lineLimit(1)
            Spacer(minLength: 0)
            MusicPlayPauseButton(player: player, side: WontonTheme.size.icon)
            Button(action: player.next) {
                Image(systemName: "forward.fill").font(.title3)
            }
            .buttonStyle(.plain)
            .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
            .accessibilityLabel(String(localized: .Music0.musicNext))
        }
        .foregroundStyle(Color(colors.textPrimary))
        .padding(.leading, WontonTheme.spacing.s8)
        .padding(.vertical, WontonTheme.spacing.s4)
        .wontonGlassSurface(in: Capsule())
        .contentShape(Capsule())
        .onTapGesture { showsPlayer = true }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(String(localized: .Music0.musicNowPlaying))
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.bottom, WontonTheme.spacing.s8)
    }
}

/// 播放页：大封面、歌名、进度和三个按钮。队列页在下一步。
struct MusicNowPlayingView: View {
    let player: MusicPlayer
    @State private var scrubbing: Double?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s24) {
            Capsule()
                .fill(Color(colors.textSecondary).opacity(0.4))
                .frame(width: WontonTheme.size.grabberWidth, height: WontonTheme.size.grabberHeight)
                .padding(.top, WontonTheme.spacing.s8)
            Spacer(minLength: 0)
            if let item = player.view?.session.item {
                GeometryReader { proxy in
                    MusicArtwork(item: item, side: min(proxy.size.width, proxy.size.height))
                        .shadow(color: .black.opacity(0.25), radius: 24, y: 12)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .aspectRatio(1, contentMode: .fit)
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    Text(item.title)
                        .wontonText(WontonTheme.typography.sectionTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                    Text(item.artist ?? item.album ?? "")
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            progress
            controls
            Spacer(minLength: 0)
        }
        .padding(.horizontal, WontonTheme.spacing.s32)
        .background(Color(colors.canvas).ignoresSafeArea())
        .wontonDialog(problem: player.problem, isPresented: Binding(
            get: { player.problem != nil }, set: { if !$0 { player.problem = nil } }
        ))
    }

    private var progress: some View {
        let duration = Double(max(player.durationMs, 1))
        let position = scrubbing ?? Double(min(player.positionMs, player.durationMs))
        return VStack(spacing: WontonTheme.spacing.s4) {
            Slider(
                value: Binding(get: { position }, set: { scrubbing = $0 }),
                in: 0...duration
            ) { editing in
                if !editing, let target = scrubbing {
                    player.seek(to: UInt64(target))
                    scrubbing = nil
                }
            }
            .tint(Color(colors.textPrimary))
            HStack {
                Text(MusicFormat.clock(UInt64(position)))
                Spacer()
                Text("-" + MusicFormat.clock(UInt64(max(0, duration - position))))
            }
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(colors.textSecondary))
            .monospacedDigit()
        }
    }

    private var controls: some View {
        HStack {
            Button(action: player.previous) {
                Image(systemName: "backward.fill").font(.largeTitle)
            }
            .accessibilityLabel(String(localized: .Music0.musicPrevious))
            Spacer()
            MusicPlayPauseButton(player: player, side: 56)
            Spacer()
            Button(action: player.next) {
                Image(systemName: "forward.fill").font(.largeTitle)
            }
            .accessibilityLabel(String(localized: .Music0.musicNext))
        }
        .buttonStyle(.plain)
        .foregroundStyle(Color(colors.textPrimary))
        .padding(.horizontal, WontonTheme.spacing.s24)
    }
}

/// 播放和暂停是同一个按钮。
private struct MusicPlayPauseButton: View {
    let player: MusicPlayer
    let side: CGFloat

    var body: some View {
        Button(action: player.togglePlayPause) {
            Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                .resizable()
                .scaledToFit()
                .frame(width: side, height: side)
                .contentTransition(.symbolEffect(.replace))
        }
        .buttonStyle(.plain)
        .frame(minWidth: WontonTheme.size.target, minHeight: WontonTheme.size.target)
        .accessibilityLabel(String(localized: player.isPlaying ? .Music1.musicPause : .Music0.musicPlay))
    }
}

/// 播放中的那一项的封面：经核心找同一家提供方取，取不到画占位。
private struct MusicArtwork: View {
    let item: MediaItemRef
    let side: CGFloat
    @Environment(\.musicPlayer) private var player
    @Environment(\.displayScale) private var scale
    @Environment(\.wontonColors) private var colors
    @State private var image: UIImage?

    var body: some View {
        ZStack {
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                Color(colors.surfaceSubtle)
                WontonIcon.music.view(size: side * 0.4).foregroundStyle(Color(colors.textSecondary))
            }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: side > 100 ? WontonTheme.radii.card : WontonTheme.radii.control))
        .task(id: item.id) {
            if let cached = MusicCovers.shared.artwork(for: item) {
                image = cached
                return
            }
            image = nil
            guard let core = player?.core else { return }
            let pixels = UInt32(max(side, 300) * scale)
            if let bytes = await core.mediaArtwork(item: item, width: pixels), let loaded = UIImage(data: bytes) {
                MusicCovers.shared.store(loaded, for: item)
                image = loaded
            }
        }
    }
}
