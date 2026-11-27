import AVFoundation
import Observation
import SwiftUI
import WontonClient

/// 原生音乐的播放器（ADR-0113 第 4 节）：会话、队列、换曲都在核心，这里照播放视图出声，播放器发生的事报回核心
/// （报的节奏、放完换下一首还是停由核心定）。App 里只有一个，离开音乐页也接着放。
@MainActor
@Observable
final class MusicPlayer {
    /// 这个人当前的播放；还没有会话时为空，停了的会话也留着（显示停在哪）。
    private(set) var view: PlaybackViewModel?
    private(set) var isPlaying = false
    private(set) var positionMs: UInt64 = 0
    /// 起播、出声没成：弹一句。
    var problem: Problem?
    @ObservationIgnored let core: ClientCore
    @ObservationIgnored private let player = AVPlayer()
    @ObservationIgnored private var loaded: Loaded?
    @ObservationIgnored private var itemObservers: [NSObjectProtocol] = []
    @ObservationIgnored private var started = false
    @ObservationIgnored private lazy var system = MusicSystemControls(player: self)
    @ObservationIgnored private lazy var audioSession = MusicAudioSessionWatch(player: self)

    /// 播放器里装着的是哪条会话的哪一项：两样都没变就不重开流。
    private struct Loaded: Equatable {
        let session: String
        let item: String
    }

    init(core: ClientCore) {
        self.core = core
    }

    /// 报这台设备能直放什么、开始跟着播放视图走。重复调没事。
    func start() {
        guard !started else { return }
        started = true
        audioSession.start()
        // 只报音频：视频的播放器另报，后报的覆盖先报的，所以这里把 iOS 原生能解的都写上。
        _ = try? core.reportPlayerSupport(support: PlayerSupport(
            mimeTypes: ["audio/mpeg", "audio/aac", "audio/mp4", "audio/flac", "audio/x-flac", "audio/wav",
                        "audio/x-wav", "audio/aiff", "audio/x-m4a", "audio/alac"],
            containers: ["mp3", "aac", "m4a", "mp4", "flac", "wav", "aiff", "caf"],
            maxBitrate: 0,
            hdr: false
        ))
        player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 1), queue: .main) { [weak self] time in
            MainActor.assumeIsolated { self?.tick(time) }
        }
        let subscription = core.subscribePlayback()
        Task { [weak self] in
            while let next = await subscription.next() {
                guard let self else { return }
                await self.apply(next)
            }
        }
    }

    /// 点了音乐页上的一首歌：从这首起放，没成的原因弹出来。
    func play(_ subscription: MusicPageSubscription, songId: String) {
        start()
        Task {
            do {
                try await subscription.play(songId: songId)
            } catch let error as MediaError {
                problem = error.problem
            } catch {
                problem = MediaFailure(code: .other(error.localizedDescription), message: "", retryAfterSecs: nil)
                    .problem
            }
        }
    }

    var title: String? { view.map(\.session.item.title) }

    var durationMs: UInt64 {
        if let planned = view?.plan?.durationMs, planned > 0 { return planned }
        return view?.session.item.durationMs ?? 0
    }

    /// 有会话、没停：迷你播放条和播放页才出现。
    var isActive: Bool { view.map { $0.session.stopReason == nil } ?? false }

    func togglePlayPause() {
        if isPlaying { pause() } else { resume() }
    }

    func pause() {
        player.pause()
        isPlaying = false
        report(.paused(positionMs: currentMs))
        control(.pause)
    }

    func resume() {
        player.play()
        isPlaying = true
        control(.resume)
    }

    func next() { control(.next) }

    /// 放过三秒按“上一首”是回到开头，和系统播放器一样。
    func previous() {
        if currentMs > 3_000 {
            seek(to: 0)
        } else {
            control(.prev)
        }
    }

    func seek(to positionMs: UInt64) {
        player.seek(to: CMTime(value: CMTimeValue(positionMs), timescale: 1000))
        self.positionMs = positionMs
        report(.seeked(positionMs: positionMs))
        system.update()
    }

    private var currentMs: UInt64 {
        let seconds = player.currentTime().seconds
        return seconds.isFinite && seconds > 0 ? UInt64(seconds * 1000) : 0
    }

    private func tick(_ time: CMTime) {
        guard loaded != nil, player.rate > 0 else { return }
        positionMs = currentMs
        report(.playing(positionMs: positionMs))
        // 锁屏和控制中心的进度跟着走：系统按速率外推，拖动、缓冲以后会偏。
        system.update()
    }

    private func report(_ event: PlayerEvent) {
        if case let .seekTo(position) = core.reportPlayer(event: event) {
            player.seek(to: CMTime(value: CMTimeValue(position), timescale: 1000))
            player.play()
        }
    }

    private func control(_ ctl: ControlOp) {
        guard let session = view?.session, session.stopReason == nil else { return }
        do {
            try core.dispatch(action: .media(action: .control(session: session.id, ctl: ctl)))
        } catch {
            problem = MediaFailure(code: .transportUnavailable, message: "\(error)", retryAfterSecs: nil).problem
        }
    }

    /// 播放视图来了：会话停了就停声并补报最后的位置，换了会话或条目就重开流，再照核心的状态放或停。
    private func apply(_ next: PlaybackViewModel) async {
        view = next
        if let failure = next.failure { problem = failure.problem }
        let session = next.session
        guard session.stopReason == nil else {
            stopHere()
            return
        }
        guard next.plan != nil else { return }
        let wanted = Loaded(session: session.id, item: session.item.id)
        if loaded != wanted {
            await load(session, as: wanted)
        }
        switch core.playerState().phase {
        case .playing:
            if player.rate == 0 { player.play() }
            isPlaying = true
        case .paused:
            player.pause()
            isPlaying = false
        case .buffering, .stopped:
            break
        }
        system.update()
    }

    private func load(_ session: PlaybackSession, as wanted: Loaded) async {
        let address: String
        do {
            address = try await core.playbackStreamUrl(session: session.id)
        } catch let error as MediaError {
            problem = error.problem
            return
        } catch {
            return
        }
        guard let url = URL(string: address) else { return }
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .default)
        try? AVAudioSession.sharedInstance().setActive(true)
        let item = AVPlayerItem(url: url)
        watch(item)
        player.replaceCurrentItem(with: item)
        loaded = wanted
        positionMs = session.positionMs
        if session.positionMs > 0 {
            await player.seek(to: CMTime(value: CMTimeValue(session.positionMs), timescale: 1000))
        }
        system.loadArtwork(for: session.item)
    }

    /// 会话在别处停了（另一台设备接着放、Agent 停了）或者这里放完了：停声，最后的位置报一次。
    private func stopHere() {
        guard loaded != nil else { return }
        player.pause()
        report(.stopped(positionMs: currentMs))
        player.replaceCurrentItem(with: nil)
        loaded = nil
        isPlaying = false
        system.clear()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    /// 放到头、放不下去：`AVPlayer` 不抛错，看通知和条目的状态。
    private func watch(_ item: AVPlayerItem) {
        itemObservers.forEach(NotificationCenter.default.removeObserver)
        let center = NotificationCenter.default
        itemObservers = [
            center.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.finished() }
            },
            center.addObserver(
                forName: .AVPlayerItemFailedToPlayToEndTime, object: item, queue: .main
            ) { [weak self] note in
                let error = note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
                MainActor.assumeIsolated { self?.failed(error) }
            }
        ]
    }

    private func finished() {
        report(.finished(positionMs: durationMs))
    }

    private func failed(_ error: Error?) {
        let message = error?.localizedDescription ?? ""
        // 回环连不上是传输断了；连上了还放不了多半是格式。
        let code: MediaFailureCode = (error as? URLError) != nil ? .transportUnavailable : .unsupportedByDevice
        isPlaying = false
        report(.failed(positionMs: currentMs, code: code, message: message))
    }
}

extension EnvironmentValues {
    @Entry var musicPlayer: MusicPlayer?
    /// 点了这一页上的一首歌（歌的 id）；没有播放器时为空，歌不能点。
    @Entry var musicPlay: ((String) -> Void)?
}
