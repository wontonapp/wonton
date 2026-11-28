import SwiftUI
import WontonClient

extension MainView {
    /// 插件页层：系统的 NavigationStack 供给左边缘右滑的交互式返回；路径空时透明且不挡下面的根页面。
    @ViewBuilder var pageStack: some View {
        NavigationStack(path: $pagePath) {
            Color.clear
                .background { PageStackBack() }
                .navigationDestination(for: PageRoute.self) { route in
                    switch route {
                    case let .tab(tab):
                        if let open = pages.viewModel.stack.first(where: { $0.tab == tab }) {
                            pluginPage(open)
                        } else {
                            // 核心已把这页弹走：保活的会话还在，等这一帧过渡结束。
                            Color.clear.toolbar(.hidden, for: .navigationBar)
                        }
                    case let .music(music):
                        MusicPageScreen(route: music, context: musicContext(music.pluginId))
                    }
                }
        }
        .allowsHitTesting(pagePath.isEmpty ? false : pages.showsPluginPage)
    }

    @ViewBuilder private func pluginPage(_ entry: OpenPage) -> some View {
        // 原生的音乐库只接就绪的入口页；启动中、已停止这些状态照旧由网页页的状态页说明。
        if entry.page.native == .music, case .ready = entry.page.state, case let .plugin(pluginId, _) = entry.tab {
            MusicLibraryRoot(
                tab: entry.tab, pluginId: pluginId, title: entry.page.title, context: musicContext(pluginId),
                pages: pages, plugins: plugins, chat: chat, open: open
            )
            .id(entry.tab)
        } else {
            webPage(entry).toolbar(.hidden, for: .navigationBar)
        }
    }

    private func musicContext(_ pluginId: String) -> MusicContext {
        MusicContext(
            core: pages.core,
            pluginId: pluginId,
            push: { pagePath.append(.music($0)) },
            replaceTop: { route in
                if case .music = pagePath.last { pagePath[pagePath.count - 1] = .music(route) }
            }
        )
    }

    @ViewBuilder private func webPage(_ entry: OpenPage) -> some View {
        TabPageView(
            pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
            tab: entry.tab, placed: .page(tab: entry.tab), store: pages.webViews,
            page: entry.page, draft: receiveDraft, open: open,
            pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom
        )
        .id(entry.tab)
    }
}
