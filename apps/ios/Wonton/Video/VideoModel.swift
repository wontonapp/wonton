import AVFoundation
import Foundation
import Observation
import WontonClient

/// 影视页的状态：首页、海报墙、详情都由客户端核翻好（`video_*`），这里只管取、记住和把动作转回去。
/// `source` 是打开的那个媒体源插件：首页、库、搜索和类型都只问它；条目自己带来源，往里点按条目的走。
@MainActor
@Observable
final class VideoModel {
    let core: ClientCore
    let source: String?
    /// 当前在放的（整屏或画中画）。
    private var player: VideoPlayerController?
    /// 点了没成的动作（标看过、收藏、下载、起播），弹一个对话框说原因。
    var actionProblem: Problem?
    /// 每有一次播放停下并报给了核心就加一：看过、接着看的位置变了，详情页据此重取。
    private(set) var playbackEnded = 0
    private var reportedSupport = false
    /// 每点一次播放加一；在路上的起播回来时号对不上（又点了一次、离开了详情）就作废。
    private var playTicket = 0

    init(core: ClientCore, source: String?) {
        self.core = core
        self.source = source
    }

    func home() async -> Result<VideoHome, MediaError> {
        await capture { try await self.core.videoHome(source: self.source) }
    }

    func wall(_ query: VideoWallQuery) async -> Result<VideoWall, MediaError> {
        await capture { try await self.core.videoWall(query: query) }
    }

    func detail(_ id: String, source: String?) async -> Result<VideoDetail, MediaError> {
        await capture { try await self.core.videoDetail(id: id, source: source) }
    }

    func genres(library: String?) async -> Result<[String], MediaError> {
        await capture { try await self.core.videoGenres(library: library, source: self.source) }
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

    /// 失败了的下载再来一次，接着上次下到的地方。上一次还没回来时客户端核不再开一个。
    func retryDownload(_ itemId: String) async {
        do {
            try await core.videoRetryDownload(itemId: itemId)
        } catch {
            actionProblem = error.videoProblem
        }
    }

    /// 暂停了的下载接着下。
    func resumeDownload(_ itemId: String) async {
        do {
            try await core.videoResumeDownload(itemId: itemId)
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

    /// 在路上的起播作废：回来的会话停掉，不弹播放器。
    func abandonPendingPlay() {
        playTicket += 1
    }

    /// 起播：有写完的离线副本放本机文件，否则让核心开会话。在放的先关掉、等它的界面收完。
    func play(_ item: MediaItemRef, title: String, from position: UInt64, audio: UInt16?, subtitle: UInt16?) async {
        playTicket += 1
        let ticket = playTicket
        if let old = player {
            player = nil
            await old.shut()
        }
        guard ticket == playTicket else { return }
        reportSupport()
        if case let .ready(path) = core.videoOffline(itemId: item.id) {
            let local = URL(fileURLWithPath: path)
            let player = VideoPlayerController(core: core, local: local, title: title, from: position)
            open(player)
            attachSession(to: player, item, from: position, audio: audio, subtitle: subtitle)
            return
        }
        do {
            let playback = try await core.videoPlay(item: item, positionMs: position, audio: audio, subtitle: subtitle)
            guard ticket == playTicket else {
                // 又点了一次或离开了：这个会话没人放，停掉。
                try? await core.videoStop(session: playback.session, positionMs: position, finished: false)
                return
            }
            guard let player = VideoPlayerController(core: core, playback: playback, title: title) else {
                actionProblem = Problem(title: String(localized: .Video2.videoPlaybackFailed), detail: playback.url)
                return
            }
            open(player)
        } catch {
            if ticket == playTicket { actionProblem = error.videoProblem }
        }
    }

    /// 放离线副本也尽量开个会话，好记接着看的位置和看完。播放器先放着不等它：连不上家里时开会话要等到超时。
    /// 会话回来时播放器还在就挂上，已经关了就停掉；开不成就不记。
    private func attachSession(
        to player: VideoPlayerController,
        _ item: MediaItemRef,
        from position: UInt64,
        audio: UInt16?,
        subtitle: UInt16?
    ) {
        let core = core
        Task {
            let playback = try? await core.videoPlay(item: item, positionMs: position, audio: audio, subtitle: subtitle)
            guard let session = playback?.session else { return }
            player.attach(session: session)
        }
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
        // 带上编解码（RFC 6381），提供方才知道哪些能直放；只报容器的话一律转码。AVPlayer 在 iOS 17 支持的机型上
        // 都能硬解 HEVC。
        let codecs = "avc1,hvc1,mp4a,ac-3,ec-3,alac,flac,mp3"
        let support = PlayerSupport(
            mimeTypes: ["video/mp4", "video/quicktime", "video/x-m4v"].map { "\($0); codecs=\"\(codecs)\"" }
                + ["audio/mp4", "audio/aac", "audio/mpeg"],
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
