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
    /// 这一条的颜色：核心的取色脚本报上来的“页面最顶上实际显示的颜色”（`ClientCore.pageColorScript`，两端共用），
    /// 脚本没报或报了没有可用的颜色时是 App 底色。注入了脚本的页不拿 WebKit 的 `themeColor` 垫：单页应用画出顶栏之前
    /// 它就是页面声明的那个颜色，和顶栏对不上时顶条会先闪一下它（脚本为此在页面加载完之前不报声明的颜色）。
    /// 公网网页不注入脚本（ADR-0052），用 WebKit 给的，照 Safari 的读法：页面声明的 `theme-color`，没声明就是 `html`、
    /// `body` 的背景色，都没有时是网页视图自己的底色（我们设成了 App 底色）。WebKit 的两个属性都能 KVO。
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
        topColor = reportsTopColor ? reportedTopColor : webView.themeColor ?? webView.underPageBackgroundColor
    }

    /// 取色脚本给谁注入：让出顶条的整页和内容窗口，公网网页除外（它没有注入脚本，ADR-0052）。
    var reportsTopColor: Bool {
        if case .panel = placed { return false }
        if case .url = tab {
            #if DEBUG
            return Self.urlAsAppPage
            #else
            return false
            #endif
        }
        return true
    }

    #if DEBUG
    /// 假数据没有隧道，整页里加载得出来的只有公网网页；设了这个，它照应用页排（没有胶囊、注入取色脚本），
    /// 走查才看得到真网页在应用页上的位置和顶条的颜色。
    static let urlAsAppPage = ProcessInfo.processInfo.environment["WONTON_UI_URL_AS_APP_PAGE"] == "1"
    #endif

    /// 建 WebView 时装通道。插件页的脚本在 `configure` 里和别的用户脚本一起装（那里会先清空）；
    /// 走查里照应用页排的公网网页不走那一段，脚本在这里装。
    func installTopColorChannel(_ controller: WKUserContentController) {
        guard reportsTopColor else { return }
        controller.add(WeakTabMessageHandler(self), name: "wontonPageColor")
        if case .url = tab { controller.addUserScript(topColorUserScript()) }
    }

    func removeTopColorChannel() {
        topColorObservations = []
        topColor = nil
        reportedTopColor = nil
        guard reportsTopColor else { return }
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: "wontonPageColor")
    }

    /// 核心的取色脚本经 window.wontonPageColor 报颜色（同 Android 的 WebMessageListener 名字），垫到 iOS 的消息处理上。
    func topColorUserScript() -> WKUserScript {
        let channel = """
        window.wontonPageColor = {
          postMessage: (message) => window.webkit.messageHandlers.wontonPageColor.postMessage(message),
        };
        """
        return WKUserScript(
            source: channel + "\n" + core.pageColorScript(), injectionTime: .atDocumentStart, forMainFrameOnly: true
        )
    }

    /// 页面报来的 `#rrggbb`；空串是没有可用的颜色，别的写法一律当没有（页面能往这个通道里发任何东西）。
    /// 只认主框架、页面自己的源（同媒体通道）。
    func receiveTopColor(_ message: WKScriptMessage, view: WKWebView) {
        guard message.frameInfo.isMainFrame, let body = message.body as? String,
              Self.sameOrigin(message.frameInfo.securityOrigin, of: view) else { return }
        reportedTopColor = Self.pageColor(body)
        readTopColor()
    }

    private static func pageColor(_ text: String) -> UIColor? {
        let digits = text.dropFirst()
        guard text.hasPrefix("#"), digits.count == 6, digits.allSatisfy(\.isHexDigit),
              let value = UInt32(digits, radix: 16) else { return nil }
        return UIColor(
            red: CGFloat((value >> 16) & 0xFF) / 255, green: CGFloat((value >> 8) & 0xFF) / 255,
            blue: CGFloat(value & 0xFF) / 255, alpha: 1
        )
    }
}
