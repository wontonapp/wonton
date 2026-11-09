import SwiftUI
import WontonClient

/// 插件图标，统一裁成圆角方形（ADR-0017：图标满幅，不自己画圆角）。没有图标、取不到时画同一个占位（ADR-0026）。
/// 还在取的时候只画底色：路径慢时要等一会儿，画占位看着就像这个插件没有图标。
struct PluginIconView: View {
    let icon: PluginIcon?
    let size: CGFloat
    let model: PluginsModel
    @State private var image: UIImage?
    @State private var isLoading = true
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * WontonTheme.ratio.pluginIconRadius, style: .continuous)
        ZStack {
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
                    .frame(width: size, height: size)
                    .clipShape(shape)
                    .overlay(shape.strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline))
            } else if isLoading {
                shape.fill(Color(colors.surfaceSubtle)).frame(width: size, height: size)
            } else {
                WontonIcon.appsFill.view(size: size * 0.54)
                    .foregroundStyle(Color(colors.accentInkOnFill))
                    .frame(width: size, height: size)
                    .accentFill(shape)
            }
        }
        .task(id: source?.path) {
            // 取不到的图标画占位（ADR-0017），不当作错误。
            guard let source else {
                image = nil
                isLoading = false
                return
            }
            isLoading = true
            image = try? await model.image(source)
            isLoading = false
        }
    }

    /// 列表和主屏用小图，详情和确认页用大图。
    private var source: PluginImage? {
        size > 96 ? icon?.large : icon?.small
    }
}
