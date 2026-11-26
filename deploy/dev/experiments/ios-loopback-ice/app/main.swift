import UIKit
import WebKit

// Minimal harness: one full-screen WKWebView, page URL from SIMCTL_CHILD_PAGE_URL.
// No native UI, no media capture — the page alone drives the WebRTC experiment.
final class Host: UIViewController, WKNavigationDelegate {
    override func loadView() {
        let cfg = WKWebViewConfiguration()
        cfg.mediaTypesRequiringUserActionForPlayback = []
        let web = WKWebView(frame: .zero, configuration: cfg)
        web.navigationDelegate = self
        web.translatesAutoresizingMaskIntoConstraints = false
        view = UIView()
        view.backgroundColor = .white
        view.addSubview(web)
        NSLayoutConstraint.activate([
            web.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            web.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            web.topAnchor.constraint(equalTo: view.topAnchor),
            web.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        let raw = ProcessInfo.processInfo.environment["PAGE_URL"] ?? "http://127.0.0.1:8000/index.html"
        NSLog("load %@", raw)
        web.load(URLRequest(url: URL(string: raw)!))
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        NSLog("loaded %@", webView.url?.absoluteString ?? "?")
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        NSLog("fail %@", error.localizedDescription)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        NSLog("provisional-fail %@", error.localizedDescription)
    }
}

@main
final class App: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(_ app: UIApplication, didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        let w = UIWindow(frame: UIScreen.main.bounds)
        w.rootViewController = Host()
        w.makeKeyAndVisible()
        window = w
        return true
    }
}
