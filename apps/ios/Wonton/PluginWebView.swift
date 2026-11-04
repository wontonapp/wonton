import Network
import SwiftUI
import WebKit
import WontonClient

/// 和 `wonton-tunnel` 的 `ERROR_HEADER` 一致。
private let tunnelErrorHeader = "x-wonton-tunnel-error"

/// 插件网页。WKWebView 的 HTTP 代理设为核心的本地代理并带上本次的会话凭据，`Host` 原样保留，
/// 插件地址和 OIDC 签发方地址都和服务器上一致。
///
/// iOS 17 的 `proxyConfigurations` 只支持 HTTP CONNECT 代理：每条连接先 `CONNECT`，之后是原样的
/// HTTP 请求。`matchDomains` 只放家里服务器的基础域名：它和它的子域名走代理，CDN、字体等外部资源直连。
struct PluginWebView: UIViewRepresentable {
    let url: URL
    let proxy: ProxySettings
    /// 加一就重新加载。
    let reload: Int
    let onLoaded: () -> Void
    let onFailed: () -> Void

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        context.coordinator.update(from: self)
        #if DEBUG
        // 调试时可以用 Mac 上 Safari 的 Web 检查器看网页。
        webView.isInspectable = true
        // 真机测前后台切换时 WONTON_DEV_RELOAD_ON_FOREGROUND=1：每次回到前台重新加载，验证本地代理和隧道
        // 在挂起后还能用，不用点按。
        if ProcessInfo.processInfo.environment["WONTON_DEV_RELOAD_ON_FOREGROUND"] != nil {
            context.coordinator.reloadOnForeground(webView)
        }
        #endif
        load(in: webView, coordinator: context.coordinator)
        return webView
    }

    /// 核心换了服务器或重建了本地代理时，端口和凭据会变，旧代理上的连接已经断了：换上新设置重新加载。
    func updateUIView(_ webView: WKWebView, context: Context) {
        let coordinator = context.coordinator
        coordinator.update(from: self)
        guard coordinator.loadedURL != url || coordinator.proxy != proxy || coordinator.reload != reload else {
            return
        }
        load(in: webView, coordinator: coordinator)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    private func load(in webView: WKWebView, coordinator: Coordinator) {
        guard let port = NWEndpoint.Port(rawValue: proxy.port) else { return }
        let endpoint = NWEndpoint.hostPort(host: NWEndpoint.Host(proxy.host), port: port)
        var configuration = ProxyConfiguration(httpCONNECTProxy: endpoint)
        configuration.applyCredential(username: proxy.username, password: proxy.password)
        configuration.matchDomains = [proxy.domain]
        webView.configuration.websiteDataStore.proxyConfigurations = [configuration]
        coordinator.proxy = proxy
        coordinator.loadedURL = url
        coordinator.reload = reload
        webView.load(URLRequest(url: url))
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var proxy: ProxySettings?
        var loadedURL: URL?
        var reload = 0
        private var onLoaded: () -> Void = {}
        private var onFailed: () -> Void = {}

        func update(from view: PluginWebView) {
            onLoaded = view.onLoaded
            onFailed = view.onFailed
        }

        /// 本地代理连不上隧道时回的错误页不给主人看，也不算加载完成（ADR-0032）。插件自己的 502 不带这个响应头，照常显示。
        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationResponse: WKNavigationResponse,
            decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void
        ) {
            let response = navigationResponse.response as? HTTPURLResponse
            if navigationResponse.isForMainFrame, response?.value(forHTTPHeaderField: tunnelErrorHeader) != nil {
                decisionHandler(.cancel)
                onFailed()
            } else {
                decisionHandler(.allow)
            }
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            onLoaded()
            #if DEBUG
            Self.log("网页加载完成 page=\(Self.withoutQuery(webView.url)) title=\(webView.title ?? "-")")
            #endif
        }

        func webView(
            _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
        ) {
            failed(error)
        }

        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            failed(error)
        }

        private func failed(_ error: Error) {
            // 新的加载取代了旧的，或者上面取消了响应（WebKit 的 102），不是失败。
            let error = error as NSError
            let replaced = error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled
            let policy = error.domain == "WebKitErrorDomain" && error.code == 102
            if !replaced, !policy { onFailed() }
            #if DEBUG
            Self.log("网页加载失败：\(Self.describe(error))")
            #endif
        }

        #if DEBUG
        private weak var reloadingWebView: WKWebView?

        /// 只在从后台回来时触发（启动时不发这个通知）。用 selector 注册：协调器释放时系统自动注销。
        func reloadOnForeground(_ webView: WKWebView) {
            reloadingWebView = webView
            NotificationCenter.default.addObserver(
                self, selector: #selector(appWillEnterForeground),
                name: UIApplication.willEnterForegroundNotification, object: nil
            )
        }

        @objc private func appWillEnterForeground() {
            Self.log("回到前台，重新加载网页")
            reloadingWebView?.reload()
        }

        // 开发时把加载过程打到标准错误（devicectl / simctl 的控制台看得到），真机上不用点按也能测。
        // 只记到路径：查询串和片段里可能有 OAuth 授权码等秘密；错误也不打 userInfo，里面有完整地址。
        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            Self.log("网页进程被系统结束")
        }

        /// 带 UTC 时间，和核心的日志对得上。
        private static func log(_ message: String) {
            let time = Date.now.formatted(Date.ISO8601FormatStyle(includingFractionalSeconds: true))
            FileHandle.standardError.write(Data("\(time) \(message)\n".utf8))
        }

        private static func describe(_ error: Error) -> String {
            let error = error as NSError
            return "\(error.domain) \(error.code) \(error.localizedDescription)"
        }

        private static func withoutQuery(_ url: URL?) -> String {
            guard let url, var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return "-" }
            parts.query = nil
            parts.fragment = nil
            return parts.string ?? "-"
        }
        #endif
    }
}
