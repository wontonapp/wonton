import SwiftUI
import WontonClient

/// 插件图标，统一裁成圆角方形（ADR-0017：图标满幅，不自己画圆角）。没有图标、取不到时画按分类选的占位（ADR-0026）。
struct PluginIconView: View {
    let icon: PluginIcon?
    let category: PluginCategory?
    let size: CGFloat
    let model: PluginsModel
    @State private var image: UIImage?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * WontonTheme.ratio.pluginIconRadius, style: .continuous)
        ZStack {
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
                    .frame(width: size, height: size)
                    .clipShape(shape)
                    .overlay(shape.strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline))
            } else {
                category.glyph.view(size: size * 0.54)
                    .foregroundStyle(Color(colors.accentInkOnFill))
                    .frame(width: size, height: size)
                    .accentFill(shape)
            }
        }
        .task(id: source?.path) {
            // 取不到的图标画占位（ADR-0017），不当作错误。
            guard let source else {
                image = nil
                return
            }
            image = try? await model.image(source)
        }
    }

    /// 列表和主屏用小图，详情和确认页用大图。
    private var source: PluginImage? {
        size > 96 ? icon?.large : icon?.small
    }
}

private extension PluginCategory? {
    var glyph: WontonIcon {
        switch self {
        case .ai: .cpuFill
        case .agent: .agentFill
        case .photos: .imageFill
        case .media: .musicFill
        case .files: .folderFill
        case .documents: .documentFill
        case .productivity: .tasksFill
        case .communication: .chatFill
        case .home: .homeFill
        case .network: .globeFill
        case .security: .shieldFill
        case .developer: .codeFill
        case .downloads: .downloadFill
        case .tools: .wrenchFill
        case .system: .settingsFill
        case .other, .none: .appsFill
        }
    }
}
