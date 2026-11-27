import AVFoundation
import Foundation
import Observation
import WontonClient

/// 影视页的状态：首页、海报墙、详情都由客户端核翻好（`video_*`），这里只管取、记住和把动作转回去。
@MainActor
@Observable
final class VideoModel {
    let core: ClientCore
    /// 当前在放的（整屏或画中画）。
    private var player: VideoPlayerController?
    /// 点了没成的动作（标看过、收藏、下载、起播），弹一个对话框说原因。
    var actionProblem: Problem?
    /// 每有一次播放停下并报给了核心就加一：看过、接着看的位置变了，详情页据此重取。
    private(set) var playbackEnded = 0
    private var reportedSupport = false

    init(core: ClientCore) {
        self.core = core
    }

    func home() async -> Result<VideoHome, MediaError> {
        await capture { try await self.core.videoHome(source: nil) }
    }

    func wall(_ query: VideoWallQuery) async -> Result<VideoWall, MediaError> {
        await capture { try await self.core.videoWall(query: query) }
    }

    func detail(_ id: String, source: String?) async -> Result<VideoDetail, MediaError> {
        await capture { try await self.core.videoDetail(id: id, source: source) }
    }

    func genres(library: String?) async -> Result<[String], MediaError> {
        await capture { try await self.core.videoGenres(library: library, source: nil) }
    }

    /// 标看过或收藏；部分没成的也算没成，原因弹出来。成了返回真。
    func setPlayed(_ entry: VideoEntry, _ played: Bool) async -> Bool {
        await act { try await self.core.videoSetPlayed(ids: [entry.id], played: played, source: entry.source) }
    }

    func setFavorite(_ entry: VideoEntry, _ favorite: Bool) async -> Bool {
        await act { try await self.core.videoSetFavorite(ids: [entry.id], favorite: favorite, source: entry.source) }
    }

    func download(_ item: MediaItemRef) async {
        do {
            _ = try await core.videoDownload(item: item)
        } catch {
            actionProblem = error.videoProblem
        }
    }

    func offline(_ itemId: String) -> OfflineState? {
        core.videoOffline(itemId: itemId)
    }

    /// 失败了的下载再来一次，接着上次下到的地方。
    func retryDownload(_ itemId: String) async {
        do {
            _ = try await core.videoRetryDownload(itemId: itemId)
        } catch {
            actionProblem = error.videoProblem
        }
    }

    func removeOffline(_ itemId: String) {
        do {
            try core.videoRemoveOffline(itemId: itemId)
        } catch {
            actionProblem = error.videoProblem
        }
    }

    /// 起播：有写完的离线副本放本机文件，否则让核心开会话。在放的先关掉、等它的界面收完。
    func play(_ item: MediaItemRef, title: String, from position: UInt64, audio: UInt16?, subtitle: UInt16?) async {
        if let old = player {
            player = nil
            await old.shut()
        }
        reportSupport()
        if case let .ready(path) = core.videoOffline(itemId: item.id) {
            let session = await offlineSession(item, from: position)
            let local = URL(fileURLWithPath: path)
            open(VideoPlayerController(core: core, local: local, session: session, title: title, from: position))
            return
        }
        do {
            let playback = try await core.videoPlay(item: item, positionMs: position, audio: audio, subtitle: subtitle)
            guard let player = VideoPlayerController(core: core, playback: playback, title: title) else {
                actionProblem = Problem(title: String(localized: .Video2.videoPlaybackFailed), detail: playback.url)
                return
            }
            open(player)
        } catch {
            actionProblem = error.videoProblem
        }
    }

    /// 放离线副本也尽量开个会话，好记接着看的位置和看完；几秒内开不成（连不上家里）就不记，照放。
    private func offlineSession(_ item: MediaItemRef, from position: UInt64) async -> String? {
        let core = core
        let attempt = Task {
            try await core.videoPlay(item: item, positionMs: position, audio: nil, subtitle: nil).session
        }
        let timer = Task {
            try await Task.sleep(for: .seconds(3))
            attempt.cancel()
        }
        defer { timer.cancel() }
        return try? await attempt.value
    }

    private func open(_ player: VideoPlayerController) {
        self.player = player
        player.onClose = { [weak self, weak player] in
            if let self, self.player === player { self.player = nil }
        }
        player.onEnded = { [weak self] error in
            guard let self else { return }
            if let error {
                actionProblem = error.videoProblem
            } else {
                playbackEnded += 1
            }
        }
        player.onFailure = { [weak self] reason in
            self?.actionProblem = Problem(title: String(localized: .Video2.videoPlaybackFailed), detail: reason)
        }
        player.present()
    }

    /// 告诉核心 AVPlayer 能直放什么；起播前报一次，核心按它让提供方选直放还是转码。
    private func reportSupport() {
        guard !reportedSupport else { return }
        let support = PlayerSupport(
            mimeTypes: ["video/mp4", "video/quicktime", "video/x-m4v", "audio/mp4", "audio/aac", "audio/mpeg"],
            containers: ["mp4", "m4v", "mov"],
            maxBitrate: 0,
            hdr: AVPlayer.eligibleForHDRPlayback
        )
        reportedSupport = (try? core.reportPlayerSupport(support: support)) != nil
    }

    private func capture<T>(_ body: @escaping () async throws -> T) async -> Result<T, MediaError> {
        do {
            return .success(try await body())
        } catch let error as MediaError {
            return .failure(error)
        } catch {
            return .failure(.Failed(failure: WontonClient.MediaFailure(
                code: .other("\(error)"), message: "\(error)", retryAfterSecs: nil
            )))
        }
    }

    private func act(_ body: @escaping () async throws -> VideoActionResult) async -> Bool {
        do {
            let result = try await body()
            if let failed = result.failed.first {
                actionProblem = MediaError.Failed(failure: failed.failure).videoProblem
                return false
            }
            return true
        } catch {
            actionProblem = error.videoProblem
            return false
        }
    }
}

extension Error {
    /// 媒体的失败按类别说人话；技术细节收进“详情”。
    var videoProblem: Problem {
        guard case let .Failed(failure) = self as? MediaError else { return problem }
        return failure.videoProblem
    }
}

extension WontonClient.MediaFailure {
    var videoProblem: Problem {
        let detail = message
        switch code {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep),
                detail: detail
            )
        case .permissionDenied:
            return Problem(
                title: String(localized: .Problem3.problemForbiddenTitle),
                nextStep: String(localized: .Problem3.problemForbiddenNextStep),
                detail: detail
            )
        case .pluginUnavailable:
            return Problem(
                title: String(localized: .Video1.videoProblemNoServiceTitle),
                nextStep: String(localized: .Video1.videoProblemNoServiceNextStep),
                detail: detail
            )
        case .sourceRequired:
            return Problem(
                title: String(localized: .Video1.videoProblemManySourcesTitle),
                nextStep: String(localized: .Video0.videoProblemManySourcesNextStep),
                detail: detail
            )
        case .authenticationRequired:
            return Problem(
                title: String(localized: .Video1.videoProblemSignInTitle),
                nextStep: String(localized: .Video2.videoProblemSignInNextStep),
                detail: detail
            )
        case .mediaGone:
            return Problem(title: String(localized: .Video2.videoProblemGoneTitle), detail: detail)
        case .unsupportedByDevice:
            return Problem(title: String(localized: .Video0.videoProblemUnsupportedTitle), detail: detail)
        case .localStorageFull:
            return Problem(
                title: String(localized: .Video2.videoProblemPhoneFullTitle),
                nextStep: String(localized: .Video1.videoProblemPhoneFullNextStep),
                detail: detail
            )
        default:
            return Problem(
                title: String(localized: .Problem2.problemUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: detail
            )
        }
    }
}
