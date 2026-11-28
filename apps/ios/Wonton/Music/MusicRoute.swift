import SwiftUI
import WontonClient

/// 音乐库里推进去的一页：插件和页。
struct MusicRoute: Hashable {
    let pluginId: String
    let page: MusicPage
}

/// 插件页层路径上的一项：核心页面栈里的页，或者某页里推进去的音乐页。音乐页只在壳里，跟着它前面那一页走。
enum PageRoute: Hashable {
    case tab(TabRef)
    case music(MusicRoute)
}

extension [PageRoute] {
    /// 路径里核心页面栈的那部分。
    var tabs: [TabRef] {
        compactMap { route in
            if case let .tab(tab) = route { tab } else { nil }
        }
    }

    /// 核心的页面栈变了：每页后面推进去的音乐页跟着它留下，页没了就一起去掉。
    func aligned(to stack: [TabRef]) -> [PageRoute] {
        var followers: [TabRef: [PageRoute]] = [:]
        var owner: TabRef?
        for route in self {
            switch route {
            case let .tab(tab):
                owner = tab
                followers[tab] = []
            case .music:
                if let owner { followers[owner, default: []].append(route) }
            }
        }
        return stack.flatMap { [PageRoute.tab($0)] + (followers[$0] ?? []) }
    }
}

/// 音乐页共用的：核心、哪个插件，和往页面层推下一页。
struct MusicContext {
    let core: ClientCore
    let pluginId: String
    let push: (MusicRoute) -> Void
    /// 换掉最上面这一页（专辑换排序）：不多一层返回。
    let replaceTop: (MusicRoute) -> Void

    func open(_ page: MusicPage) {
        push(MusicRoute(pluginId: pluginId, page: page))
    }

    func replace(_ page: MusicPage) {
        replaceTop(MusicRoute(pluginId: pluginId, page: page))
    }
}

extension EnvironmentValues {
    @Entry var music: MusicContext?
}
