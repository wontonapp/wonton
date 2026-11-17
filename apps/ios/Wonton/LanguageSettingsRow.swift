import SwiftUI

/// 设置首页的“语言”行：右边是 App 在用的语言的自称（ADR-0043）。改语言要去系统设置里本 App 的页面，
/// 跳出去之前就地说明去哪、点什么、改完会怎样（wonton#686：直接跳走，测试者不知道下一步在哪）。
struct LanguageSettingsRow: View {
    @Environment(\.openURL) private var openURL
    @State private var showsHint = false

    var body: some View {
        Button {
            showsHint = true
        } label: {
            WontonRow(title: String(localized: .Settings.settingsLanguage), value: AppLanguage.selfName,
                showsForward: true)
        }
        .alert(String(localized: .Settings.settingsLanguageChangeHint), isPresented: $showsHint) {
            Button(String(localized: .Settings.appLockOpenSettings)) {
                if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
            }
            Button(String(localized: .Common.commonCancel), role: .cancel) {}
        } message: {
            Text(String(localized: .Settings.settingsLanguageChangeSteps))
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
