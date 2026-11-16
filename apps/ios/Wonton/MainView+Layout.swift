import SwiftUI
import WontonClient

extension MainView {
    @ViewBuilder var adaptiveContent: some View {
        if usesTwoColumns {
            NavigationSplitView(columnVisibility: .constant(.all)) {
                VStack(spacing: 0) {
                    wideEntries
                    if showsMyApps {
                        SidebarPanel(home: home, plugins: plugins,
                                     current: pages.viewModel.stack.last?.tab ?? pages.viewModel.root,
                                     navigate: navigate, appsOnly: true)
                            .frame(maxHeight: 320)
                    }
                    chatListPanel(listOnly: true)
                }
                .clipped()
                .accessibilityIdentifier("wide-screen-list")
                .environment(\.wontonOpenApps) { openApps() }
                .navigationSplitViewColumnWidth(min: 280, ideal: 320, max: 400)
                .toolbar(removing: .sidebarToggle)
            } detail: {
                contentLayers
                .clipped()
                .accessibilityIdentifier("wide-screen-detail")
                .toolbar(removing: .sidebarToggle)
            }
            .navigationSplitViewStyle(.balanced)
        } else {
            SidebarContainer(isOpen: $showsSidebar, edgeSwipe: allowsEdgeSwipe) {
                contentLayers
                .environment(\.wontonOpenSidebar) { showsSidebar = true }
            } panel: {
                SidebarPanel(home: home, plugins: plugins,
                             current: pages.viewModel.stack.last?.tab ?? pages.viewModel.root, navigate: navigate)
            }
        }
    }

    /// 主界面叠着的三层，窄屏和宽屏的右栏都是它：根页面、“全部应用”、插件页。
    var contentLayers: some View {
        ZStack {
            // 被上面一层整个盖住时不留在辅助功能里：出错页的底色不是辅助功能元素，旁白和界面测试会穿过去
            // 落到下面聊天页的按钮上。
            rootContent
                .accessibilityHidden(showsApps || pages.showsPluginPage)
            appsLayer
            pageStack
        }
    }

    /// “全部应用”这一层（网格、商店、插件详情），配对后才有：和插件页层一样是盖在根页面上的导航栈，
    /// 栈空时透明且不挡下面，所以左边缘右滑是系统的交互式返回，设置的盖子也照常从主界面弹。
    @ViewBuilder private var appsLayer: some View {
        if isPaired {
            HomeView(
                model: home, plugins: plugins, chat: chat, settings: settings, open: open,
                pendingCards: chat.viewModel.pendingConfirmations, openCardRoom: openCardRoom,
                showSettings: { settings.path = []; showsSettings = true },
                showServer: { showServerRoute(.server) }, settingsPresented: showsSettings || !showsApps,
                layered: true, devWebURL: devWebURL, path: $appsPath
            )
            .allowsHitTesting(showsApps)
            .accessibilityHidden(!showsApps || pages.showsPluginPage)
        }
    }

    private var wideEntries: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            WontonIconButton(icon: .compose, label: String(localized: .Chat1.chatNewChat)) { navigate(.newChat) }
            WontonIconButton(icon: .apps, label: String(localized: .Sidebar.sidebarMyApps)) {
                showsMyApps.toggle()
            }
            .accessibilityAddTraits(showsMyApps ? .isSelected : [])
            WontonIconButton(icon: .store, label: String(localized: .Store.storeTitle)) { navigate(.store) }
            WontonIconButton(icon: .settings, label: TabText.settings) { navigate(.settings) }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, WontonTheme.spacing.s8)
    }
}
