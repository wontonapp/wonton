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
    var rejection: Problem?
    /// 页面出现时自动发的刷新被核心拒绝的原因。不弹窗，没有 `viewModel.problem` 时就地显示。
    private(set) var refreshRejection: CoreError?

    let core: ClientCore
    /// 核心给的图片地址内容不变（ADR-0017），按地址缓存解码后的图片。
    @ObservationIgnored private let images = NSCache<NSString, UIImage>()

    init(core: ClientCore) {
        self.core = core
        viewModel = core.plugins()
    }

    func isWorking(on id: String) -> Bool {
        viewModel.tasks.contains { $0.id == id && $0.state == .running }
    }

    /// 失败的任务里，插件包在确认之后变了的要重新确认。
    func reconfirmation(for task: PluginTask, source: String? = nil) -> PluginConfirmation? {
        switch task.operation {
        case .install:
            let source = source ?? viewModel.store?.allPlugins.first { $0.id == task.id }?.source
            return source.map { .install(source: $0) }
        case .upgrade:
            return .upgrade(id: task.id)
        case .uninstall:
            return nil
        }
    }

    /// 在主界面的 `.task` 里调用，App 在前台时一直订阅。
    func observe() async {
        for await viewModel in core.subscribePlugins().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }

    /// 发页面出现时的刷新，被拒绝的原因记在 `refreshRejection`。
    func refresh(_ action: Action) {
        do {
            try core.dispatch(action: action)
            refreshRejection = nil
        } catch {
            refreshRejection = error as? CoreError ?? .Upstream(reason: String(describing: error))
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
    case web(tab: TabRef)
}

extension InstalledPlugin {
    /// 打开它的方式，第一种是点图标时用的。都没有时点开看详情。
    var entries: [PluginEntry] {
        var entries: [PluginEntry] = []
        if kind == .agent, let bot = botUserId { entries.append(.chat(botUserId: bot)) }
        if let first = tabChoices.first { entries.append(.web(tab: first.tab)) }
        return entries
    }

    var isBusy: Bool {
        switch status {
        case .installing, .upgrading, .uninstalling:
            return true
        case .running, .stopped, .degraded, .unknown:
            return false
        }
    }
}

/// 插件详情页：商店里的（来源最新的版本）或者装着的。
enum PluginRoute: Hashable {
    case store(source: String)
    case installed(id: String)

    /// 核心认得的同一页。
    var coreRoute: PluginDetailRoute {
        switch self {
        case let .store(source): .store(source: source)
        case let .installed(id): .installed(id: id)
        }
    }
}

/// 安装或升级的确认页。
enum PluginConfirmation: Identifiable, Equatable {
    case install(source: String)
    case upgrade(id: String)
    /// 主人选的压缩包。
    case upload(archive: Data)
    /// 换到主人填的来源。
    case upgradeFrom(id: String, source: String)
    /// 换到主人选的压缩包。
    case upgradeUpload(id: String, archive: Data)
    /// 核心取好预览、请主人拿主意的一键操作（ADR-0017）：预览不再重新取。
    case store(StoreConfirmation)

    var id: String {
        switch self {
        case let .install(source):
            return "install:\(source)"
        case let .upgrade(id):
            return "upgrade:\(id)"
        case let .upload(archive):
            return "upload:\(archive.hashValue)"
        case let .upgradeFrom(id, source):
            return "upgrade:\(id):\(source)"
        case let .upgradeUpload(id, archive):
            return "upgrade:\(id):\(archive.hashValue)"
        case let .store(confirmation):
            return "store:\(confirmation.isUpgrade ? "upgrade" : "install"):\(confirmation.preview.id)"
        }
    }

    /// 核心已经取好的预览；确认页不再自己取。
    var preloaded: InstallPreview? {
        if case let .store(confirmation) = self { return confirmation.preview }
        return nil
    }

    var isUpgrade: Bool {
        switch self {
        case .upgrade, .upgradeFrom, .upgradeUpload: return true
        case .install, .upload: return false
        case .store(let confirmation): return confirmation.isUpgrade
        }
    }

    /// 主人选的版本可能比装着的旧，不叫升级。
    var actionTitle: String {
        switch self {
        case .install, .upload: return String(localized: .Install.installActionInstall)
        case .upgrade: return String(localized: .Install.installActionUpgrade)
        case .upgradeFrom, .upgradeUpload: return String(localized: .Install.installChangeVersion)
        case let .store(confirmation):
            return String(
                localized: confirmation.isUpgrade
                    ? .Install.installActionUpgrade
                    : .Install.installActionInstall
            )
        }
    }
}

extension StoreListing {
    /// 商店里全部的插件，按显示的顺序。
    var allPlugins: [StoreItem] {
        official.plugins + added.flatMap(\.section.plugins) + community.plugins
    }
}

/// 要卸载的插件，确认对话框用。
struct UninstallTarget: Identifiable {
    let id: String
    let title: String
}

extension StoreConfirmation {
    var preview: InstallPreview {
        switch self {
        case let .install(preview), let .upgrade(preview): return preview
        }
    }

    var isUpgrade: Bool {
        if case .upgrade = self { return true }
        return false
    }
}
