import Observation
import SwiftUI
import UIKit
import WontonClient

/// 一页音乐内容：订阅核心的那一页，壳只画 `view`。
@MainActor
@Observable
final class MusicPageModel {
    private(set) var view: MusicPageView
    /// 收藏没成功：弹一句。
    var actionProblem: Problem?
    let pluginId: String
    @ObservationIgnored private let core: ClientCore
    @ObservationIgnored let subscription: MusicPageSubscription

    /// 订阅跟着模型活：页面盖住、露出来不重订（重订就是重取一遍），弹出页面模型释放时取消。
    init(core: ClientCore, pluginId: String, page: MusicPage) {
        self.core = core
        self.pluginId = pluginId
        let subscription = core.subscribeMusicPage(pluginId: pluginId, page: page)
        self.subscription = subscription
        view = subscription.view()
        Task { [weak self] in
            while let next = await subscription.next() {
                guard let self else { return }
                self.view = next
            }
        }
    }

    deinit { subscription.cancel() }

    func act(_ action: MusicPageAction) { subscription.act(action: action) }

    func star(_ target: MusicTarget, _ starred: Bool) {
        Task {
            if let problem = await core.musicAct(pluginId: pluginId, action: .star(target: target, starred: starred)) {
                actionProblem = problem.problem
            }
        }
    }
}

/// 封面图：按插件、封面、边长缓存在内存里，盘上的缓存在核心。
@MainActor
final class MusicCovers {
    static let shared = MusicCovers()
    private let images = NSCache<NSString, UIImage>()

    func cached(_ key: String) -> UIImage? { images.object(forKey: key as NSString) }

    func load(core: ClientCore, pluginId: String, cover: String, pixels: UInt32) async -> UIImage? {
        let key = "\(pluginId)/\(cover)/\(pixels)"
        if let image = cached(key) { return image }
        guard let bytes = await core.musicCover(pluginId: pluginId, cover: cover, size: pixels),
              let image = UIImage(data: bytes) else { return nil }
        images.setObject(image, forKey: key as NSString)
        return image
    }

    /// 播放中那一项的封面（经核心找提供方取的），按条目记。
    func artwork(for item: MediaItemRef) -> UIImage? { cached("item/\(item.id)") }

    func store(_ image: UIImage, for item: MediaItemRef) {
        images.setObject(image, forKey: "item/\(item.id)" as NSString)
    }
}

extension MusicProblem {
    /// 类别由核心给，文字按类别写在这里；原样的原因收进详情。
    var problem: Problem {
        switch failure {
        case .transport:
            Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep),
                detail: details
            )
        case .pluginNotReady:
            Problem(
                title: String(localized: .Music1.musicProblemNotReadyTitle),
                nextStep: String(localized: .Music0.musicProblemNotReadyNextStep),
                detail: details
            )
        case .notSignedIn:
            Problem(
                title: String(localized: .Music0.musicProblemNotSignedInTitle),
                nextStep: String(localized: .Music0.musicProblemNotSignedInNextStep),
                detail: details
            )
        case .permissionDenied:
            Problem(
                title: String(localized: .Music1.musicProblemForbiddenTitle),
                nextStep: String(localized: .Music1.musicProblemForbiddenNextStep),
                detail: details
            )
        case .notFound:
            Problem(
                title: String(localized: .Music0.musicProblemNotFoundTitle),
                nextStep: String(localized: .Music0.musicProblemNotFoundNextStep),
                detail: details
            )
        case .upstream:
            Problem(
                title: String(localized: .Music1.musicProblemUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: details
            )
        case .protocol:
            Problem(
                title: String(localized: .Music0.musicProblemProtocolTitle),
                nextStep: String(localized: .Problem0.problemPluginProtocolNextStep),
                detail: details
            )
        case .other:
            Problem(
                title: String(localized: .Music0.musicProblemUnknownTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: details
            )
        }
    }
}
