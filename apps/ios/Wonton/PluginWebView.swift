import Network
import SwiftUI
import WebKit
import WontonClient

/// 插件网页。WKWebView 的 HTTP 代理设为核心的本地代理并带上本次的会话凭据，`Host` 原样保留，
/// 插件地址和 OIDC 签发方地址都和服务器上一致。
///
/// iOS 17 的 `proxyConfigurations` 只支持 HTTP CONNECT 代理：每条连接先 `CONNECT`，之后是原样的
/// HTTP 请求。`matchDomains` 只放家里服务器的基础域名：它和它的子域名走代理，CDN、字体等外部资源直连。
struct PluginWebView: UIViewRepresentable {
    let url: URL
    let proxy: ProxySettings

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        #if DEBUG
        // 调试时可以用 Mac 上 Safari 的 Web 检查器看网页。
        webView.isInspectable = true
        #endif
        load(in: webView, coordinator: context.coordinator)
        return webView
    }

    /// 核心换了服务器或重建了本地代理时，端口和凭据会变，旧代理上的连接已经断了：换上新设置重新加载。
    func updateUIView(_ webView: WKWebView, context: Context) {
        let coordinator = context.coordinator
        guard coordinator.loadedURL != url || coordinator.proxy != proxy else { return }
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
        webView.load(URLRequest(url: url))
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var proxy: ProxySettings?
        var loadedURL: URL?

        #if DEBUG
        // 开发时把加载结果打到标准错误（devicectl / simctl 的控制台看得到），真机上不用点按也能测。
        // 只记到路径：查询串和片段里可能有 OAuth 授权码等秘密；错误也不打 userInfo，里面有完整地址。
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            let page = Self.withoutQuery(webView.url)
            FileHandle.standardError.write(Data("网页加载完成 page=\(page) title=\(webView.title ?? "-")\n".utf8))
        }

        func webView(
            _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
        ) {
            let error = error as NSError
            let line = "网页加载失败：\(error.domain) \(error.code) \(error.localizedDescription)\n"
            FileHandle.standardError.write(Data(line.utf8))
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
