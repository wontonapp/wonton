import SwiftUI

extension EnvironmentValues {
    @Entry var wontonGlassControls = false
    @Entry var wontonSystemToolbar = false
}

extension View {
    /// 复合浮层只铺一层玻璃；内部按钮仍用原生语义，不各叠一层材质。
    @ViewBuilder func wontonGlassSurface<S: Shape>(in shape: S) -> some View {
        if #available(iOS 26, *) {
            glassEffect(.regular, in: shape)
        } else {
            background(.regularMaterial, in: shape)
        }
    }

    func wontonBarAction() -> some View {
        modifier(WontonBarActionStyle())
    }

    @ViewBuilder func wontonNavigationAction() -> some View {
        if #available(iOS 26, *) {
            buttonStyle(.glass).controlSize(.large)
        } else {
            buttonStyle(.plain)
        }
    }
}

private struct WontonBarActionStyle: ViewModifier {
    @Environment(\.wontonSystemToolbar) private var systemToolbar

    @ViewBuilder func body(content: Content) -> some View {
        if #available(iOS 26, *), systemToolbar {
            content.buttonStyle(.automatic)
        } else if #available(iOS 26, *) {
            content.buttonStyle(.glass).controlSize(.large)
        } else {
            content.buttonStyle(WontonSmallButtonStyle())
        }
    }
}
