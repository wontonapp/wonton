import SwiftUI

extension EnvironmentValues {
    /// 图标画彩色分层还是单色，整个 App 一个取值。单色是同一套图形的模板版，颜色跟 `foregroundStyle`。
    @Entry var wontonIconMono = false
}

extension WontonIcon {
    /// 默认 24。读屏的名字由外层的控件给。
    func view(size: CGFloat = WontonTheme.size.icon) -> some View {
        WontonIconView(icon: self, size: size)
    }
}

private struct WontonIconView: View {
    let icon: WontonIcon
    let size: CGFloat
    @Environment(\.wontonIconMono) private var mono

    /// 画布四周给投影留了边，图形只占中间：按上游的说法，28 的图才等于 24 的图标。
    /// 多出来的边不占布局，投影可以画到格子外面。
    private static let canvasScale: CGFloat = 28.0 / 24.0

    var body: some View {
        Image(icon.assetName + (mono ? "Mono" : "Color"))
            .resizable()
            .renderingMode(mono ? .template : .original)
            .frame(width: size * Self.canvasScale, height: size * Self.canvasScale)
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}
