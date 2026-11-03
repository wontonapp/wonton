import SwiftUI
import WontonClient

extension Color {
    init(_ color: Rgb) {
        self.init(
            .sRGB,
            red: Double(color.red) / 255,
            green: Double(color.green) / 255,
            blue: Double(color.blue) / 255
        )
    }
}

private struct WontonColorsKey: EnvironmentKey {
    static var defaultValue: ThemeColors { defaultAppearance().light }
}

extension EnvironmentValues {
    /// 核心推导好的颜色令牌，按系统的明暗选好了一套（ADR-0026）。
    var wontonColors: ThemeColors {
        get { self[WontonColorsKey.self] }
        set { self[WontonColorsKey.self] = newValue }
    }
}

/// 把外观交给下面的所有界面。主按钮和链接用 `primary`：重点色只留给选中、自己的气泡、占位图和角标。
struct WontonTheme<Content: View>: View {
    let appearance: AppearanceViewModel
    @ViewBuilder let content: Content
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let colors = colorScheme == .dark ? appearance.dark : appearance.light
        content
            .environment(\.wontonColors, colors)
            .tint(Color(colors.primary))
    }
}
