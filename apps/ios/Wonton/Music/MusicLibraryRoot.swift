import SwiftUI
import WontonClient

/// 点开音乐库插件的那一页（`TabPage.native` 是音乐）：原生的首页，右上角“更多”里有网页版、加进我的应用和详情。
struct MusicLibraryRoot: View {
    let tab: TabRef
    let pluginId: String
    let title: String
    let context: MusicContext
    let pages: PagesModel
    let plugins: PluginsModel
    let chat: ChatModel
    let open: (PluginEntry) -> Void
    @State private var showsDetails = false

    var body: some View {
        MusicPageScreen(
            route: MusicRoute(pluginId: pluginId, page: .home),
            context: context,
            root: MusicRootActions(
                title: title,
                openWeb: { pages.open(.pluginPath(pluginId: pluginId, path: "/")) },
                addToMyApps: {
                    pages.send(.pages(action: .addMyApp(app: .page(tab: tab), title: nil, index: nil)))
                },
                details: { showsDetails = true }
            )
        )
        .sheet(isPresented: $showsDetails) {
            NavigationStack {
                PluginDetailView(model: plugins, chat: chat, route: .installed(id: pluginId)) { entry in
                    showsDetails = false
                    open(entry)
                }
            }
        }
    }
}
