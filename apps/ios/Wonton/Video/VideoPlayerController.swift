import AVFoundation
import AVKit
import Foundation
import SwiftUI
import WontonClient

/// 一次播放：AVPlayer、系统的播放器界面和核心的会话。位置每秒交给核心（核心按节奏报给服务器），暂停、拖动后
/// 马上报；关掉时报停在哪。放本机离线副本时没有会话，不报。
///
/// 播放器界面用系统的 `AVPlayerViewController` 整屏弹出：拖动、音轨字幕菜单（HLS 和带多轨的原片由 AVKit 列）、
/// AirPlay、画中画和关闭按钮都是它的。
@MainActor
final class VideoPlayerController: NSObject, @preconcurrency AVPlayerViewControllerDelegate {
    let player: AVPlayer
    let title: String
    private let controller = AVPlayerViewController()
    private let core: ClientCore
    private let session: String?
    private let start: UInt64
    private var timeObserver: Any?
    private var observations: [NSKeyValueObservation] = []
    private var tokens: [NSObjectProtocol] = []
    private var closed = false
    /// 画中画开着：播放器界面收起了，声音和小窗还在。
    private var inPictureInPicture = false
    /// 关掉以后交回 `VideoModel` 清理。
    var onClose: (() -> Void)?
    /// 放不了：AVPlayer 不报错只是不动，看到失败就关掉播放器、把原因交出去。
    var onFailure: ((String) -> Void)?

    /// 核心给的地址拼不成 URL 时为空，调用方按放不了处理。
    init?(core: ClientCore, playback: VideoPlayback, title: String) {
        guard let url = URL(string: playback.url) else { return nil }
        self.core = core
        self.title = title
        session = playback.session
        start = playback.positionMs
        player = AVPlayer(url: url)
        super.init()
        prepare()
    }

    init(core: ClientCore, local: URL, title: String, from position: UInt64) {
        self.core = core
        self.title = title
        session = nil
        start = position
        player = AVPlayer(url: local)
        super.init()
        prepare()
    }

    private func prepare() {
        // 静音开关打开时也要有声音；后台和画中画靠这个类别（工程的后台模式里有 audio）。
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
        try? AVAudioSession.sharedInstance().setActive(true)
        player.allowsExternalPlayback = true
        // 锁屏、控制中心和画中画显示的片名。
        let name = AVMutableMetadataItem()
        name.identifier = .commonIdentifierTitle
        name.value = title as NSString
        player.currentItem?.externalMetadata = [name]
        if start > 0 {
            let time = CMTime(value: CMTimeValue(start), timescale: 1000)
            player.seek(to: time, toleranceBefore: .zero, toleranceAfter: .zero)
        }
        let second = CMTime(value: 1, timescale: 1)
        timeObserver = player.addPeriodicTimeObserver(forInterval: second, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.report(force: false) }
        }
        observations.append(player.observe(\.timeControlStatus) { [weak self] player, _ in
            let paused = player.timeControlStatus == .paused
            Task { @MainActor in if paused { self?.report(force: true) } }
        })
        if let item = player.currentItem {
            observations.append(item.observe(\.status) { [weak self] item, _ in
                guard item.status == .failed else { return }
                let reason = item.error?.localizedDescription ?? ""
                Task { @MainActor in self?.fail(reason) }
            })
            observe(.AVPlayerItemTimeJumped, item) { [weak self] _ in self?.report(force: true) }
            observe(.AVPlayerItemDidPlayToEndTime, item) { [weak self] _ in self?.close(finished: true) }
            observe(.AVPlayerItemFailedToPlayToEndTime, item) { [weak self] note in
                let error = note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
                self?.fail(error?.localizedDescription ?? "")
            }
        }
        player.play()
    }

    /// 主线程上收这一项的通知。
    private func observe(
        _ name: Notification.Name,
        _ item: AVPlayerItem,
        _ handle: @escaping @MainActor (Notification) -> Void
    ) {
        tokens.append(NotificationCenter.default.addObserver(forName: name, object: item, queue: .main) { note in
            MainActor.assumeIsolated { handle(note) }
        })
    }

    private var positionMs: UInt64 {
        let seconds = player.currentTime().seconds
        guard seconds.isFinite, seconds > 0 else { return 0 }
        return UInt64(seconds * 1000)
    }

    private func report(force: Bool) {
        guard let session, !closed else { return }
        let core = core
        let position = positionMs
        Task { try? await core.videoProgress(session: session, positionMs: position, force: force) }
    }

    private func fail(_ reason: String) {
        guard !closed else { return }
        onFailure?(reason)
        controller.dismiss(animated: true)
        close(finished: false)
    }

    /// 整屏弹出系统的播放器界面。
    func present() {
        controller.player = player
        controller.allowsPictureInPicturePlayback = true
        controller.canStartPictureInPictureAutomaticallyFromInline = true
        controller.delegate = self
        controller.modalPresentationStyle = .fullScreen
        guard controller.presentingViewController == nil, let top = Self.topController() else { return }
        top.present(controller, animated: true)
    }

    private static func topController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        var top = scenes.flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
        while let presented = top?.presentedViewController { top = presented }
        return top
    }

    func playerViewControllerShouldAutomaticallyDismissAtPictureInPictureStart(
        _ controller: AVPlayerViewController
    ) -> Bool { true }

    func playerViewControllerWillStartPictureInPicture(_ controller: AVPlayerViewController) {
        inPictureInPicture = true
    }

    func playerViewControllerDidStopPictureInPicture(_ controller: AVPlayerViewController) {
        inPictureInPicture = false
        // 小窗关掉、没回到整屏：这次播放结束。
        if self.controller.presentingViewController == nil { close(finished: false) }
    }

    func playerViewController(
        _ controller: AVPlayerViewController,
        restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler: @escaping (Bool) -> Void
    ) {
        present()
        completionHandler(true)
    }

    func playerViewController(
        _ controller: AVPlayerViewController,
        willEndFullScreenPresentationWithAnimationCoordinator coordinator: UIViewControllerTransitionCoordinator
    ) {
        coordinator.animate(alongsideTransition: nil) { [weak self] context in
            let cancelled = context.isCancelled
            MainActor.assumeIsolated {
                guard !cancelled, let self else { return }
                // 点了关闭（不是收进画中画）：这次播放结束。
                if !self.inPictureInPicture { self.close(finished: false) }
            }
        }
    }

    /// 关掉播放：停下、报停在哪，交回模型。
    func close(finished: Bool) {
        guard !closed else { return }
        closed = true
        let position = positionMs
        player.pause()
        if let timeObserver { player.removeTimeObserver(timeObserver) }
        tokens.forEach(NotificationCenter.default.removeObserver)
        observations.removeAll()
        if let session {
            let core = core
            Task { try? await core.videoStop(session: session, positionMs: position, finished: finished) }
        }
        onClose?()
    }
}
