import MediaPlayer
import UIKit
import WontonClient

/// 锁屏、控制中心、耳机和车机上的那一块：显示正在放的歌，按钮交回播放器（播放器再交给核心）。
@MainActor
final class MusicSystemControls {
    private weak var player: MusicPlayer?
    private var artwork: MPMediaItemArtwork?
    private var artworkItem: String?
    private var registered = false

    init(player: MusicPlayer) {
        self.player = player
    }

    /// 播放器的状态变了：写进系统的“正在播放”。第一次写时接上系统的按钮。
    func update() {
        guard let player, let item = player.view?.session.item else { return }
        register()
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: item.title,
            MPMediaItemPropertyPlaybackDuration: Double(player.durationMs) / 1000,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: Double(player.positionMs) / 1000,
            MPNowPlayingInfoPropertyPlaybackRate: player.isPlaying ? 1.0 : 0.0,
            MPNowPlayingInfoPropertyMediaType: MPNowPlayingInfoMediaType.audio.rawValue
        ]
        if let artist = item.artist { info[MPMediaItemPropertyArtist] = artist }
        if let album = item.album { info[MPMediaItemPropertyAlbumTitle] = album }
        if let artwork, artworkItem == item.id { info[MPMediaItemPropertyArtwork] = artwork }
        let center = MPNowPlayingInfoCenter.default()
        center.nowPlayingInfo = info
        center.playbackState = player.isPlaying ? .playing : .paused
    }

    /// 换了一首：取封面，取到了补进“正在播放”。
    func loadArtwork(for item: MediaItemRef) {
        guard artworkItem != item.id, let core = player?.core else { return }
        artworkItem = item.id
        artwork = nil
        Task {
            // 锁屏的大图按屏幕宽取一张就够。
            let pixels = UInt32(UIScreen.main.bounds.width * UIScreen.main.scale)
            guard let bytes = await core.mediaArtwork(item: item, width: pixels),
                  let image = UIImage(data: bytes), artworkItem == item.id else { return }
            artwork = Self.artwork(image)
            MusicCovers.shared.store(image, for: item)
            update()
        }
    }

    /// MediaPlayer 在它自己的后台队列上要图：这个闭包不能带主线程隔离，否则 Swift 的隔离检查当场崩
    /// （iOS 26.5 模拟器实测，崩在 dispatch_assert_queue）。
    private nonisolated static func artwork(_ image: UIImage) -> MPMediaItemArtwork {
        MPMediaItemArtwork(boundsSize: image.size) { _ in image }
    }

    func clear() {
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        MPNowPlayingInfoCenter.default().playbackState = .stopped
    }

    private func register() {
        guard !registered else { return }
        registered = true
        let commands = MPRemoteCommandCenter.shared()
        on(commands.playCommand) { $0.resume() }
        on(commands.pauseCommand) { $0.pause() }
        on(commands.togglePlayPauseCommand) { $0.togglePlayPause() }
        on(commands.nextTrackCommand) { $0.next() }
        on(commands.previousTrackCommand) { $0.previous() }
        commands.changePlaybackPositionCommand.addTarget { [weak self] event in
            let seconds = (event as? MPChangePlaybackPositionCommandEvent)?.positionTime ?? 0
            return MainActor.assumeIsolated {
                self?.run { $0.seek(to: UInt64(max(0, seconds) * 1000)) } ?? .commandFailed
            }
        }
    }

    /// 系统的按钮在主线程上回调。
    private func on(_ command: MPRemoteCommand, _ action: @escaping @MainActor (MusicPlayer) -> Void) {
        command.addTarget { [weak self] _ in
            MainActor.assumeIsolated { self?.run(action) ?? .commandFailed }
        }
    }

    private func run(_ action: @MainActor (MusicPlayer) -> Void) -> MPRemoteCommandHandlerStatus {
        guard let player, player.isActive else { return .noActionableNowPlayingItem }
        action(player)
        return .success
    }
}
