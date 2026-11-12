import SwiftUI

/// 设置首页的“语言”行：右边是 App 在用的语言的自称，点了打开系统设置里本 App 的页面（ADR-0043）。
struct LanguageSettingsRow: View {
    @Environment(\.openURL) private var openURL

    var body: some View {
        Button {
            if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
        } label: {
            WontonRow(title: String(localized: .Settings.settingsLanguage), value: AppLanguage.selfName,
                showsForward: true)
        }
    }
}

private extension AppLanguage {
    /// App 在用的语言用它自己的话叫什么（`Locale.localizedString(forIdentifier:)`），首字母大写。
    static var selfName: String {
        let tag = current
        let name = Locale(identifier: tag).localizedString(forIdentifier: tag) ?? tag
        return name.prefix(1).capitalized + name.dropFirst()
    }
}
