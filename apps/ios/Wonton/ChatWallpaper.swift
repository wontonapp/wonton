import SwiftUI
import WontonClient

/// 聊天页压在背景上的几样颜色（ADR-0109 第 9 节）。都由 ADR-0026 的令牌叠出来，壳不另算色值：
/// 核心的 `ThemeColors` 里没有聊天背景这几项，补进核心之前先在这里按固定浓度叠。
struct ChatPalette {
    let colors: ThemeColors
    let isDark: Bool

    /// 背景的底：浅色用 `surfaceSubtle`，白气泡才浮得起来；深色用 `canvas`。
    var wallBase: Color { Color(isDark ? colors.canvas : colors.surfaceSubtle) }
    var wallPattern: Color { Color(colors.textPrimary).opacity(WontonTheme.ratio.wallPattern) }
    /// 别人的气泡：浅色是白的，深色比背景亮一档。
    var incoming: Color { Color(isDark ? colors.surfaceSubtle : colors.surface) }
    /// 日期胶囊、说明行、选项按钮的半透明底，上面的字一律白色。
    var serviceFill: Color {
        isDark
            ? Color.white.opacity(WontonTheme.ratio.serviceFillDark)
            : Color.black.opacity(WontonTheme.ratio.serviceFill)
    }
    var serviceInk: Color { .white }
    /// 消息下选项按钮的底：入气泡的颜色透一点背景，字用 `textPrimary`。
    var choiceFill: Color { incoming.opacity(WontonTheme.ratio.secondaryContent) }
    /// 浮在背景上的一层（别人的气泡、选项按钮）的投影：没有重点色以后，白气泡和浅灰背景只靠它分层（ADR-0130）。
    var lift: Color { .black.opacity(isDark ? WontonTheme.ratio.shadowDark : WontonTheme.ratio.shadow) }
}

extension View {
    /// 聊天背景上浮起来的一层。
    func chatLift(_ palette: ChatPalette) -> some View {
        shadow(color: palette.lift, radius: WontonTheme.size.shadowBlur / 2, y: WontonTheme.size.shadowY)
    }
}

extension EnvironmentValues {
    /// 聊天页的颜色：跟着 `wontonColors` 和明暗走。
    var chatPalette: ChatPalette { ChatPalette(colors: wontonColors, isDark: colorScheme == .dark) }
}

/// 聊天背景（ADR-0109 第 9 节）：灰阶的底色上铺一层细线图案。图案是这里自己画的几何小件，
/// 不用任何现成的壁纸素材。
struct ChatWallpaper: View {
    @Environment(\.chatPalette) private var palette

    var body: some View {
        ZStack {
            palette.wallBase
            Canvas { context, size in
                let ink = GraphicsContext.Shading.color(palette.wallPattern)
                let stroke = StrokeStyle(lineWidth: Self.line, lineCap: .round, lineJoin: .round)
                let columns = Int(size.width / Self.cell) + 2
                let rows = Int(size.height / (Self.cell * Self.rowShare)) + 2
                for row in 0..<rows {
                    for column in 0..<columns {
                        // 位置、转角、大小都由格子的行列定：每次重画都一样，不闪。
                        let seed = row &* 31 &+ column &* 17 &+ (row &* column) % 7
                        let shift = row.isMultiple(of: 2) ? 0 : Self.cell / 2
                        let center = CGPoint(
                            x: CGFloat(column) * Self.cell + shift - Self.cell / 4 + CGFloat(seed % 9),
                            y: CGFloat(row) * Self.cell * Self.rowShare + CGFloat(seed % 11)
                        )
                        let radius = Self.glyph + CGFloat(seed % 4)
                        let turn = CGAffineTransform(translationX: center.x, y: center.y)
                            .rotated(by: CGFloat(seed % 12) * .pi / 6)
                        let path = Self.glyph(seed % Self.kinds, radius: radius).applying(turn)
                        context.stroke(path, with: ink, style: stroke)
                    }
                }
            }
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }

    private static let cell: CGFloat = 76
    private static let rowShare: CGFloat = 0.8
    private static let glyph: CGFloat = 8
    private static let line: CGFloat = 1.2
    private static let kinds = 8

    /// 以原点为中心、大小约 `radius` 的一个小件。
    private static func glyph(_ kind: Int, radius: CGFloat) -> Path {
        var path = Path()
        switch kind {
        case 0:
            path.addEllipse(in: CGRect(x: -radius, y: -radius, width: radius * 2, height: radius * 2))
        case 1:
            path.move(to: CGPoint(x: -radius, y: 0))
            path.addLine(to: CGPoint(x: radius, y: 0))
            path.move(to: CGPoint(x: 0, y: -radius))
            path.addLine(to: CGPoint(x: 0, y: radius))
        case 2:
            // 一道波浪。
            path.move(to: CGPoint(x: -radius * 1.4, y: 0))
            path.addQuadCurve(to: .zero, control: CGPoint(x: -radius * 0.7, y: -radius))
            path.addQuadCurve(to: CGPoint(x: radius * 1.4, y: 0), control: CGPoint(x: radius * 0.7, y: radius))
        case 3:
            path.move(to: CGPoint(x: 0, y: -radius))
            path.addLine(to: CGPoint(x: radius, y: radius * 0.8))
            path.addLine(to: CGPoint(x: -radius, y: radius * 0.8))
            path.closeSubpath()
        case 4:
            path.addRoundedRect(
                in: CGRect(x: -radius * 0.8, y: -radius * 0.8, width: radius * 1.6, height: radius * 1.6),
                cornerSize: CGSize(width: radius * 0.4, height: radius * 0.4)
            )
        case 5:
            // 一只馄饨：半圆的皮，底下一道边，上面三道褶。
            path.move(to: CGPoint(x: -radius * 1.2, y: radius * 0.5))
            path.addQuadCurve(to: CGPoint(x: radius * 1.2, y: radius * 0.5), control: CGPoint(x: 0, y: -radius * 1.9))
            path.closeSubpath()
            for fold: CGFloat in [-0.5, 0, 0.5] {
                path.move(to: CGPoint(x: radius * fold, y: -radius * 0.1))
                path.addLine(to: CGPoint(x: radius * fold * 1.2, y: radius * 0.2))
            }
        case 6:
            // 一对括弧。
            path.addArc(center: CGPoint(x: radius * 0.4, y: 0), radius: radius, startAngle: .degrees(130),
                        endAngle: .degrees(230), clockwise: false)
            path.addArc(center: CGPoint(x: -radius * 0.4, y: 0), radius: radius, startAngle: .degrees(-50),
                        endAngle: .degrees(50), clockwise: false)
        default:
            for dot in -1...1 {
                path.addEllipse(in: CGRect(x: CGFloat(dot) * radius - 1, y: -1, width: 2, height: 2))
            }
        }
        return path
    }
}

extension View {
    /// 聊天页浮在内容上的一层（ADR-0107）：iOS 26 是一层玻璃，更早的系统垫材质；带 `tint` 的是主要动作。
    @ViewBuilder func chatGlass<S: Shape>(in shape: S, tint: Color? = nil, interactive: Bool = false) -> some View {
        if #available(iOS 26, *) {
            glassEffect(.regular.tint(tint).interactive(interactive), in: shape)
        } else if let tint {
            background(tint, in: shape)
        } else {
            background(.regularMaterial, in: shape)
        }
    }

    /// 同一簇里几块玻璃放进一个容器（ADR-0107）；更早的系统没有这个容器。
    @ViewBuilder func chatGlassCluster() -> some View {
        if #available(iOS 26, *) {
            GlassEffectContainer(spacing: WontonTheme.spacing.s4) { self }
        } else {
            self
        }
    }
}

/// 聊天页浮着的圆钮：看上去 `size.chatControl` 大，点得到的范围是 `size.target`。标准的 `.glass` 按钮样式定不了
/// 直径，和输入框、标题胶囊对不齐，所以自己铺玻璃（ADR-0107 的“标准样式表达不了的自绘可操作件”）。
/// 有标识才往无障碍树上写：没有时不盖掉调用方自己加的标识。
private struct OptionalIdentifier: ViewModifier {
    let identifier: String?

    init(_ identifier: String?) { self.identifier = identifier }

    func body(content: Content) -> some View {
        if let identifier {
            content.accessibilityIdentifier(identifier)
        } else {
            content
        }
    }
}

struct ChatGlassButton: View {
    let icon: WontonIcon
    let label: String
    /// 主要动作（发送）用强调填充。
    var prominent = false
    /// 可达性树的稳定标识；只给界面测试要按它找的按钮。
    var identifier: String?
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            icon.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(prominent ? colors.onPrimary : colors.textPrimary))
                .frame(width: WontonTheme.size.chatControl, height: WontonTheme.size.chatControl)
                .chatGlass(in: Circle(), tint: prominent ? Color(colors.primary) : nil, interactive: true)
                .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .modifier(OptionalIdentifier(identifier))
    }
}
