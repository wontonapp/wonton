import SwiftUI
import WebKit

/// 整页的网页怎么让开状态栏加胶囊那一条（ADR-0095“铺满”）。
enum PageTopInset {
    /// 网页框从这一条下面开始，而不是铺到屏幕顶。iOS 26 之前没有 `obscuredContentInsets`，现成应用固定在右上角的
    /// 按钮只有这样才不压在胶囊底下；按钮点得到比背景铺到顶要紧。让出来的这一条由 `PageTopStrip` 涂页面的颜色。
    static let framesWebView: Bool = {
        #if DEBUG
        // 界面测试在 iOS 26 的模拟器上走一遍旧系统的排法（`PageInsetsUITests`）。
        if ProcessInfo.processInfo.environment["WONTON_UI_FRAMED_PAGE"] == "1" { return true }
        #endif
        if #available(iOS 26, *) { return false }
        return true
    }()
}

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
