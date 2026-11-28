import AVKit
import SwiftUI
import WontonClient

/// 大图页里的视频和实况照片的动态部分：地址是客户端核的本机回环（`playback`、`live` 档，`Range` 原样过），
/// 交给系统播放器。不开播放会话：相册里的短片不进队列、不跨设备接着放（ADR-0113 第 9 节）。
@MainActor
@Observable
final class GalleryMotion {
    enum State {
        case idle
        case loading
        case ready(AVPlayer)
        case failed(Problem)
    }

    private(set) var state = State.idle
    @ObservationIgnored private var task: Task<Void, Never>?

    var player: AVPlayer? {
        if case let .ready(player) = state { return player }
        return nil
    }

    var isLoading: Bool {
        if case .loading = state { return true }
        return false
    }

    var problem: Problem? {
        if case let .failed(problem) = state { return problem }
        return nil
    }

    /// 取地址并开播；`title` 是放不了时说的那句话。
    func play(id: String, rendition: PhotoRendition, core: ClientCore, muted: Bool, title: String) {
        stop()
        state = .loading
        task = Task {
            do {
                let address = try await core.photoUrl(id: id, rendition: rendition)
                guard !Task.isCancelled else { return }
                guard let url = URL(string: address) else {
                    state = .failed(Problem(title: title, detail: URLError(.badURL).localizedDescription))
                    return
                }
                let player = AVPlayer(url: url)
                player.isMuted = muted
                state = .ready(player)
                player.play()
            } catch let MediaError.Failed(failure) {
                guard !Task.isCancelled else { return }
                let problem = failure.galleryProblem
                state = .failed(Problem(title: title, nextStep: problem.title, detail: problem.detail))
            } catch {
                guard !Task.isCancelled else { return }
                state = .failed(Problem(title: title, detail: error.localizedDescription))
            }
        }
    }

    func stop() {
        task?.cancel()
        task = nil
        player?.pause()
        state = .idle
    }

    /// 松开长按：在放的停下；没放成的原因留着，等人看完关掉。
    func release() {
        if problem == nil { stop() }
    }
}

/// 只有画面的播放层（实况照片盖在静态图上放，不带控件）。
struct GalleryPlayerLayer: UIViewRepresentable {
    let player: AVPlayer

    func makeUIView(context: Context) -> PlayerView {
        let view = PlayerView()
        view.playerLayer.videoGravity = .resizeAspect
        view.isUserInteractionEnabled = false
        return view
    }

    func updateUIView(_ view: PlayerView, context: Context) {
        view.playerLayer.player = player
    }

    final class PlayerView: UIView {
        override static var layerClass: AnyClass { AVPlayerLayer.self }
        // swiftlint:disable:next force_cast
        var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }
    }
}
