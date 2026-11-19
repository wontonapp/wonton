import SwiftUI
import WontonClient

/// 日期胶囊（ADR-0109 第 4 节）：列表里跨天处的那一枚和滚动时浮在顶上的那一枚长得一样。
struct DatePill: View {
    let timestampMs: UInt64
    /// 浮在消息上面的那一枚垫系统材质，列表里的垫浅色底。
    var floats = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(TimelineRow.date(timestampMs), format: .dateTime.year().month().day())
            .wontonText(WontonTheme.typography.caption)
            .foregroundStyle(Color(floats ? colors.textPrimary : colors.textSecondary))
            .padding(.horizontal, WontonTheme.spacing.s12)
            .padding(.vertical, WontonTheme.spacing.s4)
            .background {
                if floats {
                    Capsule().fill(.regularMaterial)
                } else {
                    Capsule().fill(Color(colors.surfaceSubtle))
                }
            }
    }
}

/// 滚动时浮在列表顶部的日期：可见区最上面那条消息的日期，停下后过 `motion.datePillHold` 淡出。
struct FloatingDatePill: View {
    /// 可见区最上面那条消息的发送时间；没有消息时为空。
    let timestampMs: UInt64?
    let pin: BottomPin
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            if pin.isScrolling, let timestampMs {
                DatePill(timestampMs: timestampMs, floats: true)
                    .padding(.top, WontonTheme.spacing.s8)
                    .transition(.opacity)
                    .accessibilityIdentifier("timelineFloatingDate")
            }
        }
        .animation(
            .easeOut(duration: reduceMotion ? WontonTheme.motion.reduced : WontonTheme.motion.state),
            value: pin.isScrolling
        )
        .allowsHitTesting(false)
    }
}

/// 跳到原消息后整行背景闪一次重点色，按 `motion.jumpHighlight` 淡出（ADR-0109 第 4 节）。
struct JumpHighlight: View {
    /// 每跳到这一行一次换一个值；不是这一行时为空。
    let trigger: Int?
    @State private var isLit = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Color(colors.accentInk)
            .opacity(isLit ? WontonTheme.ratio.scrim : 0)
            .task(id: trigger) {
                guard trigger != nil else { return }
                isLit = true
                // 先亮一帧再淡出：同一轮里亮了又灭，界面上什么都看不到。
                guard (try? await Task.sleep(for: .seconds(WontonTheme.motion.feedback))) != nil else { return }
                withAnimation(.easeOut(duration: WontonTheme.motion.jumpHighlight)) { isLit = false }
            }
    }
}

/// 原消息找不到了（翻到房间开头也没有，或者服务器取不到）：一条轻提示，过一会儿自己收起。
struct JumpMissingHint: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(.Chat.roomJumpUnavailable)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s8)
            .background(.regularMaterial, in: Capsule())
            .padding(.bottom, WontonTheme.spacing.s16)
            .accessibilityIdentifier("timelineJumpUnavailable")
    }
}

/// 停在别处时右下角的“向下”（ADR-0071、ADR-0109）：圆形材质按钮，数字在箭头上方的胶囊里，没有数字就不画胶囊。
/// 点了去哪由调用方定：有跳之前的位置先回那里，没有才到底。
struct ScrollToBottomButton: View {
    let count: Int
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: WontonTheme.spacing.s4) {
                if count > 0 {
                    Text(count > Self.badgeMax ? "\(Self.badgeMax)+" : String(count))
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, WontonTheme.spacing.s8)
                        .padding(.vertical, WontonTheme.spacing.s4 / 2)
                        .background(.thinMaterial, in: Capsule())
                }
                Image(systemName: "arrow.down")
                    .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                    .background(.regularMaterial, in: Circle())
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(
            count > 0
                ? String(localized: .Chat.roomScrollToBottomCount("\(count)"))
                : String(localized: .Chat.roomScrollToBottom)
        )
        .accessibilityIdentifier("timelineDown")
        .padding(.trailing, WontonTheme.spacing.s16)
        .padding(.bottom, WontonTheme.spacing.s16)
    }

    /// 数字超过这个数就写“99+”。
    private static let badgeMax = 99
}

/// 群聊里组尾气泡旁的头像：没有头像图片的人画名字的第一个字。
struct SenderInitial: View {
    let name: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text(name.prefix(1).uppercased())
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textPrimary))
            .frame(width: WontonTheme.size.messageAvatar, height: WontonTheme.size.messageAvatar)
            .background(Circle().fill(Color(colors.surfaceSubtle)))
            .accessibilityHidden(true)
    }
}
