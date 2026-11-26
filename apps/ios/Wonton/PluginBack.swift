import Foundation
import WebKit

/// 返回这一步该做什么；顺序即优先级。
enum PluginBackTarget: Equatable {
    case closeCustomView
    case history
    case closePage
}

/// 当前页前面最近的一步历史。[sameDocument]：走回去看到的还是这一页（同一地址，或一条会自己跳回本页的重定向），
/// 网页上不会有变化。[previous] 只给日志看。
struct HistoryStep: Equatable {
    let sameDocument: Bool
    let previous: String?
}

/// 返回的去向。网页历史分两种：跨文档的一步照浏览器的语义后退；同一文档里的一步常常只是重定向、replaceState
/// 的记录，按不出反应，按没有历史处理直接关页。全屏视频先退出，页面还在。
func pluginBackTarget(customView: Bool, history: HistoryStep?) -> PluginBackTarget {
    if customView { return .closeCustomView }
    guard let history else { return .closePage }
    return history.sameDocument ? .closePage : .history
}

/// 日志里的地址：只到路径。查询串里可能有 OAuth 授权码，片段是登录跳转的一部分。
func logUrl(_ url: String) -> String {
    String(url.prefix { $0 != "?" && $0 != "#" })
}

/// 本页的历史足迹：当前地址和重定向来源，用来认出不会换页的一步。
final class PageHistory {
    private var currentUrl: String?
    /// 正在加载的文档地址；加载完和最终地址不同就是一条重定向。
    private var startedUrl: String?
    private var redirects: [String: String] = [:]

    func started(_ url: String?) {
        guard let url else { return }
        startedUrl = url
        currentUrl = url
    }

    func finished(_ url: String?) {
        guard let started = startedUrl else { return }
        startedUrl = nil
        guard let url else { return }
        let from = urlWithoutFragment(started)
        let target = urlWithoutFragment(url)
        if from != target { redirects[from] = target }
    }

    func visited(_ url: String?) {
        guard let url else { return }
        currentUrl = url
    }

    /// 当前页前面最近的一步；没有历史、或拿不到它的地址时 nil。
    func step(in webView: WKWebView?) -> HistoryStep? {
        guard let webView, webView.canGoBack,
              let previous = webView.backForwardList.backItem?.url.absoluteString else { return nil }
        return HistoryStep(sameDocument: isSameDocument(previous), previous: logUrl(previous))
    }

    /// 同一文档：上一页和当前页的地址整串一样，或上一页是一条会跳回当前页的重定向（这一支不看片段）。
    /// 只换片段的一步（hash 路由）在网页里会翻到另一页，交给网页历史。
    func isSameDocument(_ previous: String) -> Bool {
        guard let current = currentUrl else { return false }
        return previous == current
            || redirects[urlWithoutFragment(previous)] == urlWithoutFragment(current)
    }

    /// 日志用：只到路径，去掉查询串（可能有授权码）和片段。
    func pagePath() -> String? { currentUrl.map(logUrl) }

    func clear() {
        currentUrl = nil
        startedUrl = nil
        redirects.removeAll()
    }

    private func urlWithoutFragment(_ url: String) -> String { String(url.prefix { $0 != "#" }) }
}

extension TabWebSession {
    /// 系统返回和胶囊菜单的“返回”：视频全屏先退出，网页历史里会换页的一步照常后退；
    /// 同一文档里的记录（重定向、同地址）直接关页，由调用方经 [onClosePage] 摘掉这一页。
    func backOrClose(onClosePage: () -> Void) {
        // 现取一次地址再判定：hash 这类同文档导航在部分 WebKit 版本里不回调 didSameDocumentNavigation，
        // currentUrl 会停在旧地址，只换片段的一步会被误判成同一页。
        history.visited(webView?.url?.absoluteString)
        let step = history.step(in: webView)
        // iOS 的网页视频全屏由系统接管，壳里没有 customView；这个参数和 Android 的判定共用一张表。
        let target = pluginBackTarget(customView: false, history: step)
        // 复现用例靠这几项判断返回走到了哪一步；两个地址都经过 logUrl，日志里不会有授权码。
        NSLog(
            "Wonton back: target=%@ sameDocument=%@ previous=%@ page=%@",
            String(describing: target), String(describing: step?.sameDocument),
            step?.previous ?? "nil", history.pagePath() ?? "nil"
        )
        switch target {
        case .history: webView?.goBack()
        case .closePage: onClosePage()
        case .closeCustomView: break
        }
    }
}
