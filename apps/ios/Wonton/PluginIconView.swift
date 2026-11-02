import SwiftUI
import WontonClient

/// 插件图标，统一裁成圆角方形（ADR-0017：图标满幅，不自己画圆角）。没有图标、取不到时画占位。
struct PluginIconView: View {
    let icon: PluginIcon?
    let kind: PluginKind
    let size: CGFloat
    let model: PluginsModel
    @State private var image: UIImage?

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * 0.2237, style: .continuous)
        ZStack {
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                LinearGradient(colors: [Color(.systemGray4), Color(.systemGray2)], startPoint: .top, endPoint: .bottom)
                Image(systemName: kind.symbol)
                    .font(.system(size: size * 0.42, weight: .medium))
                    .foregroundStyle(.white)
            }
        }
        .frame(width: size, height: size)
        .clipShape(shape)
        .overlay(shape.strokeBorder(Color.primary.opacity(0.08), lineWidth: 0.5))
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

extension PluginKind {
    /// 占位图标上的符号。
    var symbol: String {
        switch self {
        case .app: return "square.grid.2x2.fill"
        case .agent: return "sparkles"
        case .channel: return "bubble.left.and.bubble.right.fill"
        case .model: return "cpu"
        case .system: return "gearshape.fill"
        case .other: return "puzzlepiece.extension.fill"
        }
    }
}
