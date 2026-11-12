import SwiftUI
import WontonClient

/// ADR-0026 的令牌，和 Compose 的 `WontonTheme` 同名。颜色跟着外观变，在 `\.wontonColors` 里。
enum WontonTheme {
    static let spacing = WontonSpacing.self
    static let radii = WontonRadii.self
    static let size = WontonSize.self
    static let ratio = WontonRatio.self
    static let motion = WontonMotion.self
    static let typography = WontonTextStyle.self
}

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
struct WontonThemeRoot<Content: View>: View {
    let appearance: AppearanceViewModel
    @ViewBuilder let content: Content
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.dynamicTypeSize) private var systemTextSize

    var body: some View {
        let colors = colorScheme == .dark ? appearance.dark : appearance.light
        content
            .environment(\.wontonColors, colors)
            .tint(Color(colors.primary))
            // 放在根上，系统的字号和字体环境值整个 App 都读：包括 `wontonText`、没写令牌的文字和输入框。
            .dynamicTypeSize(appearance.textSize.dynamicTypeSize ?? systemTextSize)
            .fontDesign(appearance.font.design)
            .preferredColorScheme(appearance.themeMode.colorScheme)
    }
}

extension ThemeMode {
    fileprivate var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

extension TextSize {
    /// `.large` 是系统默认的字号，Android 的 `fontScale` 按这几档的正文放大比例取值（ADR-0028）。
    fileprivate var dynamicTypeSize: DynamicTypeSize? {
        switch self {
        case .system: nil
        case .small: .small
        case .standard: .large
        case .large: .xLarge
        case .extraLarge: .xxLarge
        }
    }
}

extension FontStyle {
    var design: Font.Design? {
        switch self {
        case .default: nil
        case .rounded: .rounded
        case .serif: .serif
        case .monospaced: .monospaced
        }
    }
}

struct WontonTextStyle: Sendable {
    let size: CGFloat
    let line: CGFloat
    let weight: Font.Weight
    let relativeTo: Font.TextStyle
}

extension View {
    /// 令牌的字号和行高，跟随系统字号。
    func wontonText(_ style: WontonTextStyle) -> some View {
        modifier(WontonTextModifier(style: style))
    }
}

private struct WontonTextModifier: ViewModifier {
    private let weight: Font.Weight
    @ScaledMetric private var size: CGFloat
    @ScaledMetric private var line: CGFloat

    init(style: WontonTextStyle) {
        weight = style.weight
        _size = ScaledMetric(wrappedValue: style.size, relativeTo: style.relativeTo)
        _line = ScaledMetric(wrappedValue: style.line, relativeTo: style.relativeTo)
    }

    func body(content: Content) -> some View {
        content
            .font(.system(size: size, weight: weight))
            .lineSpacing(max(0, line - UIFont.systemFont(ofSize: size).lineHeight))
    }
}

extension WontonIcon {
    /// 模板渲染，颜色跟 `foregroundStyle`，默认 24。读屏的名字由外层的控件给。
    func view(size: CGFloat = WontonTheme.size.icon) -> some View {
        Image(rawValue)
            .resizable()
            .renderingMode(.template)
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}
