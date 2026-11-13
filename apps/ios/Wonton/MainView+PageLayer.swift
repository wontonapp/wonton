import SwiftUI
import WontonClient

extension MainView {
    /// 插件页层：系统的 NavigationStack 供给左边缘右滑的交互式返回；路径空时透明且不挡下面的根页面。
    @ViewBuilder var pageStack: some View {
        NavigationStack(path: $pagePath) {
            Color.clear
                .background { PageStackBack() }
                .navigationDestination(for: TabRef.self) { tab in
                    Group {
                        if let open = pages.viewModel.stack.first(where: { $0.tab == tab }) {
                            pluginPage(open)
                        } else {
                            // 核心已把这页弹走：保活的会话还在，等这一帧过渡结束。
                            Color.clear
                        }
                    }
                    .toolbar(.hidden, for: .navigationBar)
                }
        }
        .allowsHitTesting(pagePath.isEmpty ? false : pages.showsPluginPage)
    }

    @ViewBuilder private func pluginPage(_ entry: OpenPage) -> some View {
        TabPageView(
            pages: pages, home: home, plugins: plugins, chat: chat, appearance: appearance,
            tab: entry.tab, placed: .page(tab: entry.tab), store: pages.webViews,
            page: entry.page, draft: receiveDraft, open: open,
            pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
            close: onboardingGateway?.tab == entry.tab ? { closeGateway(entry.tab) } : nil
        )
        .id(entry.tab)
    }

    private func closeGateway(_ tab: TabRef) {
        if pages.viewModel.stack.last?.tab == tab { pages.closeCurrent() }
        onboardingGateway = nil
    }
}
