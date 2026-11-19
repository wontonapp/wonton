import SwiftUI
import WebKit

/// 网页框上面让出来的那一条：涂页面自己的颜色，和页面的顶栏连成一片；页面还没出来时是 App 底色。
struct PageTopStrip: View {
    let color: UIColor?
    let height: CGFloat
    @Environment(\.wontonColors) private var colors

    var body: some View {
        (color.map { Color(uiColor: $0) } ?? Color(colors.canvas))
            .frame(height: height)
            .frame(maxHeight: .infinity, alignment: .top)
            .ignoresSafeArea(.container)
            .accessibilityHidden(true)
    }
}

extension TabWebSession {
    /// 跟着页面的颜色，照 Safari 的读法：页面声明的 `theme-color`（`media` 分深浅色的写法由 WebKit 挑），没声明就是
    /// `html`、`body` 的背景色，都没有时 WebKit 给的是网页视图自己的底色（我们设成了 App 底色）。两个属性都能 KVO，
    /// 单页应用切路由、切深浅色时跟着变。
    func observeTopColor(of view: WKWebView) {
        topColorObservations = [
            view.observe(\.themeColor, options: [.initial]) { [weak self] _, _ in
                Task { @MainActor in self?.readTopColor() }
            },
            view.observe(\.underPageBackgroundColor) { [weak self] _, _ in
                Task { @MainActor in self?.readTopColor() }
            }
        ]
    }

    private func readTopColor() {
        guard let webView else { return }
        topColor = webView.themeColor ?? webView.underPageBackgroundColor
    }
}
