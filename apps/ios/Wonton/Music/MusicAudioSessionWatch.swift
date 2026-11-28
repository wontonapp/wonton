import AVFoundation

/// 来电、闹钟、Siri 打断和拔耳机，按系统惯例：打断开始就暂停并报核心，结束时系统说可以续
/// （`shouldResume`）且打断前在放才续；耳机、蓝牙断开（`oldDeviceUnavailable`）暂停、不自动续。
@MainActor
final class MusicAudioSessionWatch {
    private weak var player: MusicPlayer?
    private var observers: [NSObjectProtocol] = []
    /// 打断开始时在放：结束时才可能续。
    private var resumeAfterInterruption = false

    init(player: MusicPlayer) {
        self.player = player
    }

    /// 开始听音频会话的通知。重复调没事。
    func start() {
        guard observers.isEmpty else { return }
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        observers = [
            center.addObserver(
                forName: AVAudioSession.interruptionNotification, object: session, queue: .main
            ) { [weak self] note in
                let info = note.userInfo
                let type = (info?[AVAudioSessionInterruptionTypeKey] as? UInt)
                    .flatMap(AVAudioSession.InterruptionType.init(rawValue:))
                let options = (info?[AVAudioSessionInterruptionOptionKey] as? UInt)
                    .map(AVAudioSession.InterruptionOptions.init(rawValue:)) ?? []
                MainActor.assumeIsolated { self?.interrupted(type, shouldResume: options.contains(.shouldResume)) }
            },
            center.addObserver(
                forName: AVAudioSession.routeChangeNotification, object: session, queue: .main
            ) { [weak self] note in
                let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt)
                    .flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))
                guard reason == .oldDeviceUnavailable else { return }
                MainActor.assumeIsolated { self?.routeLost() }
            }
        ]
    }

    private func interrupted(_ type: AVAudioSession.InterruptionType?, shouldResume: Bool) {
        guard let player else { return }
        switch type {
        case .began:
            resumeAfterInterruption = player.isPlaying
            if player.isPlaying { player.pause() }
        case .ended:
            if shouldResume, resumeAfterInterruption { player.resume() }
            resumeAfterInterruption = false
        case .none:
            break
        @unknown default:
            break
        }
    }

    private func routeLost() {
        guard let player, player.isPlaying else { return }
        resumeAfterInterruption = false
        player.pause()
    }
}
