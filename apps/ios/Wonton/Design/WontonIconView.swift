import SwiftUI

extension EnvironmentValues {
    /// 这一片里的图标只画单色：玻璃和材质的栏、带色的实底上，彩色分层图会和底抢颜色。
    @Entry var wontonIconMono = false
}

extension View {
    func wontonIconMono() -> some View {
        environment(\.wontonIconMono, true)
    }
}

extension WontonIcon {
    /// 默认画彩色分层版；玻璃栏里和 `wontonIconMono()` 下面画单色模板，颜色跟 `foregroundStyle`。默认 24。
    /// 读屏的名字由外层的控件给。
    func view(size: CGFloat = WontonTheme.size.icon) -> some View {
        WontonIconView(icon: self, size: size)
    }
}

private struct WontonIconView: View {
    let icon: WontonIcon
    let size: CGFloat
    @Environment(\.wontonIconMono) private var mono
    @Environment(\.wontonGlassControls) private var glassControls
    @Environment(\.wontonSystemToolbar) private var systemToolbar

    /// 彩色版的画布四周给投影留了边，图形只占中间：按上游的说法，28 的图才等于 24 的图标。
    /// 多出来的边不占布局，投影可以画到格子外面。
    private static let colorCanvasScale: CGFloat = 28.0 / 24.0

    var body: some View {
        if !mono, !glassControls, !systemToolbar, let asset = WontonIcon.colorAssets[icon] {
            Image(asset)
                .resizable()
                .renderingMode(.original)
                .frame(width: size * Self.colorCanvasScale, height: size * Self.colorCanvasScale)
                .frame(width: size, height: size)
                .accessibilityHidden(true)
        } else {
            Image(icon.rawValue)
                .resizable()
                .renderingMode(.template)
                .frame(width: size, height: size)
                .accessibilityHidden(true)
        }
    }
}
