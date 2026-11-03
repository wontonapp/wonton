import Foundation
import Observation
import UIKit
import WontonClient

/// 主屏、商店和核心之间的桥：持有核心推来的插件 ViewModel，转发操作，替视图取插件的图片。
/// 这里不做任何业务判断。
@MainActor
@Observable
final class PluginsModel {
    private(set) var viewModel: PluginsViewModel
    /// 核心拒绝主人的操作的原因，由界面弹窗展示。
    var rejection: String?
    /// 页面出现时自动发的刷新被核心拒绝的原因。不弹窗，没有 `viewModel.problem` 时就地显示。
    private(set) var refreshRejection: String?

    let core: ClientCore
    /// 核心给的图片地址内容不变（ADR-0017），按地址缓存解码后的图片。
    @ObservationIgnored private let images = NSCache<NSString, UIImage>()

    init(core: ClientCore) {
        self.core = core
        viewModel = core.plugins()
    }

    /// 失败的任务里，插件包在确认之后变了的要重新确认。
    func reconfirmation(for task: PluginTask) -> PluginConfirmation? {
        switch task.operation {
        case .install:
            let item = viewModel.store.flatMap { store in
                (store.official.plugins + store.community.plugins).first { $0.id == task.id }
            }
            return item.map { .install(source: $0.source) }
        case .upgrade:
            return .upgrade(id: task.id)
        case .uninstall:
            return nil
        }
    }

    /// 在主界面的 `.task` 里调用，App 在前台时一直订阅。
    func observe() async {
        // 名字和说明按主人的语言挑。iOS 改了系统语言会重启 App，启动时告诉核心一次就够。
        refresh(.setLanguages(languages: Locale.preferredLanguages))
        for await viewModel in core.subscribePlugins().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = String(describing: error)
        }
    }

    /// 发页面出现时的刷新，被拒绝的原因记在 `refreshRejection`。
    func refresh(_ action: Action) {
        do {
            try core.dispatch(action: action)
            refreshRejection = nil
        } catch {
            refreshRejection = String(describing: error)
        }
    }

    func image(_ image: PluginImage) async throws -> UIImage {
        let key = image.path as NSString
        if let cached = images.object(forKey: key) { return cached }
        let data = try await core.loadPluginImage(image: image)
        guard let decoded = UIImage(data: data) else { throw UndecodableImage(path: image.path) }
        images.setObject(decoded, forKey: key)
        return decoded
    }
}

/// 核心给的图片解不开。
struct UndecodableImage: Error {
    let path: String
}

/// 已安装的插件从主屏和商店打开的方式（ADR-0017 的“主屏入口”）。
enum PluginEntry {
    /// 和插件的 Bot 的私聊。
    case chat(botUserId: String)
    /// 插件网页，全屏打开就是小程序。
    case web(url: URL, title: String)
}

extension InstalledPlugin {
    /// 打开它的方式，第一种是点图标时用的。都没有时点开看详情。
    var entries: [PluginEntry] {
        var entries: [PluginEntry] = []
        if kind == .agent, let bot = botUserId { entries.append(.chat(botUserId: bot)) }
        if let text = webUrl, let url = URL(string: text) { entries.append(.web(url: url, title: title)) }
        return entries
    }

    var isBusy: Bool {
        switch status {
        case .installing, .upgrading, .uninstalling:
            return true
        case .running, .degraded, .unknown:
            return false
        }
    }
}

/// 插件详情页：商店里的（来源最新的版本）或者装着的。
enum PluginRoute: Hashable {
    case store(source: String)
    case installed(id: String)
}

/// 安装或升级的确认页。
enum PluginConfirmation: Identifiable {
    case install(source: String)
    case upgrade(id: String)

    var id: String {
        switch self {
        case let .install(source):
            return "install:\(source)"
        case let .upgrade(id):
            return "upgrade:\(id)"
        }
    }
}

/// 要卸载的插件，确认对话框用。
struct UninstallTarget: Identifiable {
    let id: String
    let title: String
}
