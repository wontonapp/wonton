import SwiftUI
import WontonClient

struct TabIconView: View {
    let item: TabItem
    let selected: Bool
    let plugins: PluginsModel
    @State private var image: UIImage?
    @State private var failed = false

    var body: some View {
        Group {
            if let page = item.page {
                switch page.icon {
                case let .plugin(icon):
                    PluginIconView(icon: icon, size: WontonTheme.size.icon, model: plugins)
                case .monochrome:
                    if let image {
                        Image(uiImage: image).resizable().renderingMode(.template)
                            .scaledToFit().frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
                    } else if failed { WontonIcon.imageBroken.view() } else {
                        Color.clear.frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
                    }
                }
            } else { builtin.view() }
        }
        .task(id: source?.path) {
            guard let source else { return }
            image = nil
            failed = false
            do { image = try await plugins.image(source) } catch { failed = true }
        }
        .accessibilityHidden(true)
    }

    private var source: PluginImage? {
        if case let .monochrome(icon, filled) = item.page?.icon { return selected ? filled ?? icon : icon }
        return nil
    }

    private var builtin: WontonIcon {
        switch item.tab {
        case .chat: selected ? .chatFill : .chat
        case .apps, .plugin, .pluginPath, .bot, .url, .sharedApp, .uiResource: selected ? .appsFill : .apps
        case .settings: selected ? .settingsFill : .settings
        }
    }
}
