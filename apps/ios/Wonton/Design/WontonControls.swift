import SwiftUI
import WontonClient

/// 自己画的按钮（ADR-0026）：语义还是系统的 `Button`，命中区 48。
struct WontonButtonStyle: ButtonStyle {
    enum Kind {
        case primary
        case secondary
        case danger
    }

    var kind: Kind = .primary
    @Environment(\.wontonColors) private var colors
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        let (fill, text): (Rgb, Rgb) = switch kind {
        case .primary: (colors.primary, colors.onPrimary)
        case .secondary: (colors.surfaceSubtle, colors.textPrimary)
        case .danger: (colors.surfaceSubtle, colors.danger)
        }
        configuration.label
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(text))
            .padding(.horizontal, WontonTheme.spacing.s16)
            .frame(minHeight: WontonTheme.size.target)
            .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(fill)))
            .contentShape(Rectangle())
            .opacity(isEnabled ? (configuration.isPressed ? 0.6 : 1) : 0.4)
            .animation(.easeOut(duration: WontonTheme.motion.feedback), value: configuration.isPressed)
    }
}

/// 图标按钮：图形 24，命中区 48。
struct WontonIconButton: View {
    let icon: WontonIcon
    let label: String
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            icon.view()
                .foregroundStyle(Color(colors.textPrimary))
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// 进下一层的图标入口，样子和 `WontonIconButton` 一样。
struct WontonIconLink<Value: Hashable>: View {
    let icon: WontonIcon
    let label: String
    let value: Value
    @Environment(\.wontonColors) private var colors

    var body: some View {
        NavigationLink(value: value) {
            icon.view()
                .foregroundStyle(Color(colors.textPrimary))
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// 一组列表行的底。
struct WontonGroup<Content: View>: View {
    @ViewBuilder let content: Content
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: 0) { content }
            .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }
}

/// 列表行。点按语义由外面的 `Button` 或 `NavigationLink` 给（`.buttonStyle(.plain)`）。
struct WontonRow: View {
    let title: String
    var titleTint: Rgb?
    var value: String?
    var valueTint: Rgb?
    var badge = false
    var showsForward = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            Text(title)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(titleTint ?? colors.textPrimary))
                .layoutPriority(1)
            Spacer(minLength: 0)
            if let value {
                Text(value)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(valueTint ?? colors.textSecondary))
                    .multilineTextAlignment(.trailing)
            }
            if badge {
                Circle().fill(Color(colors.primary)).frame(width: 12, height: 12)
                    .accessibilityLabel(String(localized: .designUpdateBadge))
            }
            if showsForward {
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target)
        .contentShape(Rectangle())
    }
}

struct WontonSeparator: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Rectangle()
            .fill(Color(colors.separator))
            .frame(height: WontonTheme.size.hairline)
            .padding(.leading, WontonTheme.spacing.s16)
    }
}

/// 提示块：错误、权限和不能用的原因，要做的事放在 `actions`。
struct WontonNotice<Actions: View>: View {
    let icon: WontonIcon
    let title: String
    var isError = true
    var lines: [String] = []
    @ViewBuilder let actions: Actions
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                icon.view(size: WontonTheme.size.iconSmall)
                Text(title).wontonText(WontonTheme.typography.body)
            }
            .foregroundStyle(Color(isError ? colors.danger : colors.textPrimary))
            ForEach(lines, id: \.self) { line in
                Text(line)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            actions
        }
        .padding(WontonTheme.spacing.s12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surfaceSubtle)))
    }
}

extension WontonNotice where Actions == EmptyView {
    init(icon: WontonIcon, title: String, isError: Bool = true, lines: [String] = []) {
        self.init(icon: icon, title: title, isError: isError, lines: lines) { EmptyView() }
    }
}

extension View {
    /// 输入框的底：配 `.textFieldStyle(.plain)`。
    func wontonField() -> some View {
        modifier(WontonFieldModifier())
    }

    /// 重点色的填充：纸质的颗粒盖在颜色上，有描边令牌时加描边。
    func accentFill<S: InsettableShape>(_ shape: S, border: Bool = true) -> some View {
        modifier(AccentFillModifier(shape: shape, border: border))
    }
}

private struct WontonFieldModifier: ViewModifier {
    @Environment(\.wontonColors) private var colors
    @FocusState private var isFocused: Bool

    func body(content: Content) -> some View {
        content
            .focused($isFocused)
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.target)
            .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
            // 系统的输入框只有文字那一行接得住点按，整个底都要能点。
            .contentShape(Rectangle())
            .onTapGesture { isFocused = true }
    }
}

private struct AccentFillModifier<S: InsettableShape>: ViewModifier {
    let shape: S
    let border: Bool
    @Environment(\.wontonColors) private var colors

    func body(content: Content) -> some View {
        content.background {
            shape.fill(Color(colors.accentFill))
                .overlay { NoiseTile().clipShape(shape) }
                .overlay {
                    if border, let color = colors.accentFillBorder {
                        shape.strokeBorder(Color(color), lineWidth: WontonTheme.size.hairline)
                    }
                }
        }
    }
}

/// 噪点贴图平铺，1 个噪点像素对 1 个物理像素（ADR-0026）。
struct NoiseTile: View {
    // 打包进 App 的资源，不会缺；解码一次后不再变。
    private static let image = UIImage(contentsOfFile: Bundle.main.path(forResource: "noise", ofType: "png")!)!.cgImage!
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        Image(decorative: Self.image, scale: displayScale)
            .resizable(resizingMode: .tile)
            .allowsHitTesting(false)
    }
}

/// 标题栏要带上的状态：短的用于根页面和聊天房间副标题；长的是标题下面多的一行。
struct WontonBarStatus {
    var title: String?
    var subtitle: String?
    var line: AnyView?
}

extension EnvironmentValues {
    @Entry var wontonBarStatus = WontonBarStatus()
}

/// 根页面只显示连接状态和操作，页面名交给导航语义。
struct WontonRootBar<Trailing: View>: View {
    let title: String
    let hasActions: Bool
    @ViewBuilder let trailing: Trailing
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.wontonOpenSidebar) private var openSidebar

    private var hasButtons: Bool { hasActions || openSidebar != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if hasButtons || status.title?.isEmpty == false {
                ZStack {
                    if let title = status.title, !title.isEmpty {
                        Text(title)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textPrimary))
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, hasButtons ? WontonTheme.size.target + WontonTheme.spacing.s8 : 0)
                            .padding(.vertical, WontonTheme.spacing.s8)
                    }
                    if hasButtons {
                        HStack(spacing: 0) {
                            if let openSidebar { SidebarButton(action: openSidebar) }
                            Spacer(minLength: 0)
                            if hasActions { trailing }
                        }
                    }
                }
                .frame(maxWidth: .infinity, minHeight: hasButtons ? WontonTheme.size.topBar : 0)
            }
            if let line = status.line {
                line.padding(.bottom, WontonTheme.spacing.s8)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .frame(maxWidth: .infinity)
    }
}

/// 根页面左上角的侧边栏按钮。
private struct SidebarButton: View {
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            WontonIcon.list.view()
                .foregroundStyle(Color(colors.textPrimary))
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(String(localized: .sidebarOpen))
    }
}

/// 进入下一层的页面的标题栏：返回在前，标题居中，操作在后。
///
/// `showsStatus` 把连接状态放进副标题；`showsStatusLine` 给内容全靠连接的页面：长时间连不上时换成标题下面那一行，带下一步，
/// 不然页面只会一直转圈。
struct WontonPushedBar<Trailing: View>: View {
    let title: String
    var icon: WontonIcon = .back
    var iconLabel = String(localized: .designBack)
    var showsStatus = false
    var showsStatusLine = false
    /// 标题旁的头像（Bot 聊天的顶栏，ADR-0044）。
    var avatar: AnyView?
    @ViewBuilder let trailing: Trailing
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.dismiss) private var dismiss

    private var line: AnyView? { showsStatusLine ? status.line : nil }

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                VStack(spacing: 0) {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if let avatar { avatar }
                        Text(title)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                            .lineLimit(1)
                            .accessibilityAddTraits(.isHeader)
                    }
                    if showsStatus || showsStatusLine, line == nil, let subtitle = status.subtitle {
                        Text(subtitle)
                            .wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textSecondary))
                            .lineLimit(1)
                    }
                }
                .padding(.horizontal, WontonTheme.size.target + WontonTheme.spacing.s8)
                HStack(spacing: 0) {
                    WontonIconButton(icon: icon, label: iconLabel) { dismiss() }
                    Spacer(minLength: 0)
                    trailing
                }
                .padding(.horizontal, WontonTheme.spacing.s4)
            }
            .frame(minHeight: WontonTheme.size.topBar)
            if let line {
                line
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.bottom, WontonTheme.spacing.s8)
            }
        }
    }
}

extension WontonPushedBar where Trailing == EmptyView {
    init(title: String) {
        self.init(title: title) { EmptyView() }
    }
}

extension View {
    /// 用自己的标题栏代替系统导航栏。
    func wontonPushedScreen(_ title: String, showsStatusLine: Bool = false) -> some View {
        wontonPushedScreen(title, showsStatusLine: showsStatusLine) { EmptyView() }
    }

    func wontonPushedScreen<Trailing: View>(
        _ title: String,
        avatar: AnyView? = nil,
        showsStatus: Bool = false,
        showsStatusLine: Bool = false,
        @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        let bar = WontonPushedBar(
            title: title, showsStatus: showsStatus, showsStatusLine: showsStatusLine,
            avatar: avatar, trailing: trailing
        )
        return wontonTopBar { bar }
            .toolbar(.hidden, for: .navigationBar)
            .modifier(WontonCanvas())
    }

    /// 盖在整屏上的页面：标题栏和进入下一层的一样，返回换成关闭。
    func wontonModalScreen(_ title: String) -> some View {
        let bar = WontonPushedBar(
            title: title, icon: .close, iconLabel: String(localized: .designClose)
        ) { EmptyView() }
        return wontonTopBar { bar }
            .modifier(WontonCanvas())
    }

    /// 根页面：连接状态和操作，底栏保留在根导航层。
    func wontonRootScreen<Trailing: View>(
        _ title: String, hasActions: Bool, @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        let bar = WontonRootBar(title: title, hasActions: hasActions, trailing: trailing)
        return wontonTopBar { bar }
            .modifier(WontonTabBarSlot())
            .navigationTitle(bar.title)
            .toolbar(.hidden, for: .navigationBar)
            .modifier(WontonCanvas())
    }
}

extension View {
    /// 顶栏、底栏没有实底，内容从后面滚过去（ADR-0029）。iOS 26 起交给系统的滚动边缘效果：栏后面的内容渐隐模糊，
    /// 状态栏那一块也是；更早的系统没有这个效果，栏和状态栏垫一层系统材质，不然字压在内容上读不清。
    @ViewBuilder func wontonTopBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        if #available(iOS 26, *) {
            safeAreaBar(edge: .top, spacing: 0, content: bar)
        } else {
            safeAreaInset(edge: .top, spacing: 0) {
                bar().background(.ultraThinMaterial, ignoresSafeAreaEdges: .top)
            }
        }
    }

    @ViewBuilder fileprivate func wontonBottomBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        if #available(iOS 26, *) {
            safeAreaBar(edge: .bottom, spacing: 0, content: bar)
        } else {
            safeAreaInset(edge: .bottom, spacing: 0, content: bar)
        }
    }
}

private struct WontonTabBarSlot: ViewModifier {
    @Environment(\.wontonTabBar) private var tabBar

    func body(content: Content) -> some View {
        content.wontonBottomBar { tabBar }
    }
}

private struct WontonCanvas: ViewModifier {
    @Environment(\.wontonColors) private var colors

    func body(content: Content) -> some View {
        content.background(Color(colors.canvas).ignoresSafeArea())
    }
}

/// 隐藏系统导航栏后，SwiftUI 默认会失去边缘返回手势（ADR-0026），这里还给它。
extension UINavigationController: @retroactive UIGestureRecognizerDelegate {
    override open func viewDidLoad() {
        super.viewDidLoad()
        interactivePopGestureRecognizer?.delegate = self
    }

    public func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        viewControllers.count > 1
    }
}
