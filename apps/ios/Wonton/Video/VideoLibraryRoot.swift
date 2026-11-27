import SwiftUI
import WontonClient

/// 影视页入口页右上角“更多”里的几项：和网页的胶囊菜单一样，网页版也从这里进。
struct VideoRootActions {
    let title: String
    let openWeb: () -> Void
    let addToMyApps: () -> Void
    let details: () -> Void
}

/// 点开影视库插件的那一页（`TabPage.native` 是影视）：原生的首页，往里点的页推进插件页层的导航栈。
struct VideoLibraryRoot: View {
    let tab: TabRef
    let pluginId: String
    let title: String
    let model: VideoModel
    let front: Bool
    let push: (VideoRoute) -> Void
    let pages: PagesModel
    let plugins: PluginsModel
    let chat: ChatModel
    let open: (PluginEntry) -> Void
    @State private var showsDetails = false

    var body: some View {
        VideoHomeView(model: model, root: VideoRootActions(
            title: title,
            openWeb: { pages.open(.pluginPath(pluginId: pluginId, path: "/")) },
            addToMyApps: {
                pages.send(.pages(action: .addMyApp(app: .page(tab: tab), title: nil, index: nil)))
            },
            details: { showsDetails = true }
        ))
        .videoPage(model: model, front: front, push: push)
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

/// 插件页层里推进去的一页影视：哪个插件的、哪一页。
struct VideoPageRoute: Hashable {
    let pluginId: String
    let route: VideoRoute
}

/// 各影视库插件的模型：入口页和推进去的页共用一个（在放的播放器、看完后的重取都挂在它上面）。
/// 不参与界面观察，只在取的时候按需建。
@MainActor
final class VideoModels {
    private var models: [String: VideoModel] = [:]

    func model(core: ClientCore, pluginId: String) -> VideoModel {
        if let model = models[pluginId] { return model }
        let model = VideoModel(core: core, source: pluginId)
        models[pluginId] = model
        return model
    }
}
