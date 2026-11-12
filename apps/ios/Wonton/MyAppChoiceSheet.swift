import SwiftUI
import WontonClient

/// 多页插件挑一页加进“我的应用”；只有一个可加的页面时不开这个面板。
struct MyAppChoiceSheet: View {
    let plugin: InstalledPlugin
    let onDismiss: () -> Void
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                let mine = plugins.viewModel.sidebar.apps.compactMap(\.tab)
                let available = plugin.tabChoices.filter { choice in !mine.contains(choice.tab) }
                ForEach(available) { choice in
                    Button {
                        pages.send(.addMyApp(app: .page(tab: choice.tab), title: nil, index: nil))
                        onDismiss()
                    } label: {
                        Text(choice.title)
                            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
    }
}

/** 插件详情页里的“我的应用”一组：每页一行，加进了的给开关，没加的给按钮。 */
struct MyAppsSection: View {
    let plugin: InstalledPlugin
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins

    public var body: some View {
        DetailSection(title: TabText.myApps) {
            let mine = plugins.viewModel.sidebar.apps.compactMap(\.tab)
            ForEach(plugin.tabChoices) { choice in
                let added = mine.contains(choice.tab)
                Toggle(choice.title, isOn: Binding(
                    get: { mine.contains(choice.tab) },
                    set: { enabled in
                        if enabled {
                            pages.send(.addMyApp(app: .page(tab: choice.tab), title: nil, index: nil))
                        } else {
                            pages.send(.removeMyApp(app: .page(tab: choice.tab)))
                        }
                    }
                ))
                .toggleStyle(WontonToggleStyle())
            }
        }
    }
}

/** 装好后的完成页：把刚装的加进"我的应用"；多页的先挑一页。 */
struct MyAppAddButton: View {
    let plugin: InstalledPlugin
    @Environment(PagesModel.self) private var pages
    @Environment(PluginsModel.self) private var plugins
    @State private var choosing = false

    var body: some View {
        Button(TabText.addToMyApps) {
            let mine = plugins.viewModel.sidebar.apps.compactMap(\.tab)
            let choices = plugin.tabChoices.filter { !mine.contains($0.tab) }
            if choices.count == 1 {
                pages.send(.addMyApp(app: .page(tab: choices[0].tab), title: nil, index: nil))
            } else if !choices.isEmpty {
                choosing = true
            }
        }
        .buttonStyle(WontonSmallButtonStyle())
        .sheet(isPresented: $choosing) {
            MyAppChoiceSheet(plugin: plugin, onDismiss: { choosing = false })
        }
    }
}
