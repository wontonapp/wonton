import Foundation

/// 登录要经过的地址：认证页（`auth.<基础域名>`）和核心自己的 OIDC 路径。
/// 系统浏览器没有隧道和设备身份，走不完，所以主框架跳到这里时留在 App 里。
struct TabLoginRoute: Equatable {
    /// 家里的基础域名（`ProxySettings.domain`）；还没连上时没有。
    let domain: String?

    func contains(_ url: URL) -> Bool {
        guard let domain, let host = url.host else { return false }
        return host == "auth.\(domain)" || (host == domain && url.path.hasPrefix("/_wonton/oidc/"))
    }
}

/// 重新加载时回哪个地址。
///
/// 重新加载（核心的重载计数变了、代理换了、网页进程被杀后重建）回到正在看的那一页，不把人送回插件首页。
/// 两类地址是一次性的，重放不得，这时回入口，插件按已有的会话直接进去：
/// - 登录往返的落点：从认证页跳回插件时地址里带着授权码。插件拿它换完会话、把下一页的数据取回来以后才换地址，
///   这期间重放它，插件拿用过的授权码再换一次。Immich v3.4.1 实测（iOS 26.5）：第二次换时登录用的 Cookie
///   已经在第一次成功后清掉，页面停在登录页报 “OAuth state is missing”，其实已经登录好了。
/// - 还在登录的路上：地址里是认证服务一次性的登录挑战；回调页从认证页落回插件后、加载完成前也还在路上，
///   这时地址里已带着授权码。
struct TabReloadTarget {
    let entry: URL
    let login: TabLoginRoute
    /// 最近一次登录往返落在插件上的地址。
    let loginLanding: URL?
    /// 主框架还在登录的路上：见 `TabWebSession.inLogin`。
    let inLogin: Bool

    /// `recovery` 是加载失败或网页进程被杀时记下的地址，`current` 是 WebView 现在的地址。
    func resolve(recovery: URL?, current: URL?) -> URL {
        guard let wanted = recovery ?? current, replayable(wanted) else { return entry }
        return wanted
    }

    private func replayable(_ url: URL) -> Bool {
        // 在登录的路上，手里的地址要么是一次性的登录挑战，要么是带授权码的回调地址，重放都不得。
        !inLogin && url != loginLanding && !login.contains(url)
    }
}
