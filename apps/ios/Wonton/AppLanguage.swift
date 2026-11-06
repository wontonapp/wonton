import Foundation

/// App 在用的语言（ADR-0043）。
enum AppLanguage {
    /// 本 App 实际落到的那种语言，如 `zh-Hant`。系统语言不在支持范围里时是 `en`，而 `Locale.preferredLanguages` 还是用户要的那种。
    static var current: String {
        Bundle.main.preferredLocalizations.first ?? "en"
    }

    /// 告诉核心的语言偏好：先是 App 在用的，再接用户要的，去掉重复。
    static var preferred: [String] {
        var seen = Set<String>()
        return ([current] + Locale.preferredLanguages).filter { seen.insert($0.lowercased()).inserted }
    }
}
