import AppIntents
import Foundation
import UIKit
import WontonClient

/// 只把可发送的值交给系统；核心、UIKit 图片和可观察模型都留在 MainActor。
struct WontonPageEntity: AppEntity, Sendable {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Page"
    static let defaultQuery = WontonPageQuery()

    let id: String
    let title: String
    var iconData: Data?
    var isTemplate = false

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(
            title: "\(title)",
            image: iconData.map { DisplayRepresentation.Image(data: $0, isTemplate: isTemplate) }
        )
    }
}

struct WontonPageQuery: EntityStringQuery {
    @MainActor
    func entities(for identifiers: [WontonPageEntity.ID]) async throws -> [WontonPageEntity] {
        let pages = try await availablePages(identifiers: Set(identifiers))
        let byID = Dictionary(uniqueKeysWithValues: pages.map { ($0.id, $0) })
        return identifiers.compactMap { byID[$0] }
    }

    @MainActor
    func suggestedEntities() async throws -> [WontonPageEntity] {
        try await availablePages()
    }

    @MainActor
    func entities(matching string: String) async throws -> [WontonPageEntity] {
        try await availablePages(matching: string)
    }

    @MainActor
    private func availablePages(identifiers: Set<String>? = nil, matching: String? = nil) async throws
        -> [WontonPageEntity] {
        let navigation: AppNavigation
        do {
            navigation = try AppNavigation.requireShared()
        } catch {
            throw IntentProblem(error)
        }
        guard try await navigation.navigationReady(deadline: .seconds(15)) else { return [] }
        try await navigation.waitForInitialPlugins(deadline: .seconds(15))
        guard navigation.allowsNavigation else { return [] }
        let core = navigation.chat.core
        let view = core.pages()
        var choices = view.stack.map(PageChoice.init)
        choices += view.myAppCandidates.map(PageChoice.init)
        var seen = Set<String>()
        var pages: [WontonPageEntity] = []
        for choice in choices {
            let id = appLinkUrl(link: .tab(tab: choice.tab))
            guard seen.insert(id).inserted else { continue }
            if let identifiers, !identifiers.contains(id) { continue }
            if let matching, !choice.title.localizedStandardContains(matching) { continue }
            var page = WontonPageEntity(id: id, title: choice.title, isTemplate: choice.isTemplate)
            if let source = choice.icon {
                // 与 App 内插件图标相同：图片不可得不阻止页面候选出现。
                if let image = try? await navigation.plugins.image(source) { page.iconData = image.pngData() }
            }
            pages.append(page)
        }
        return pages
    }
}

private struct PageChoice {
    let tab: TabRef
    let title: String
    var icon: PluginImage?
    var isTemplate = false

    init(_ open: OpenPage) {
        tab = open.tab
        title = open.title
        switch open.page.icon {
        case let .plugin(plugin): icon = plugin?.small
        case let .monochrome(source, _): icon = source; isTemplate = true
        }
    }

    init(_ candidate: TabCandidate) {
        tab = candidate.tab
        if let plugin = candidate.plugin {
            title = plugin.pageTitle ?? plugin.title
            icon = plugin.icon?.small
        } else {
            title = TabText.settings
        }
    }
}
