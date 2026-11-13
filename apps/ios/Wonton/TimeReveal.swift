import SwiftUI
import UIKit

/// 左滑整列消息，露出每条的时间（学 iMessage）：时间平时不占地方，松手弹回去。
/// 只认向左的横向拖动；竖向滚动、右滑回复和边缘返回照旧。
@MainActor
@Observable
final class TimeReveal: NSObject, UIGestureRecognizerDelegate {
    /// 整列往左挪了多少，0 到 `WontonTheme.gesture.timeReveal`。
    private(set) var offset: CGFloat = 0
    @ObservationIgnored private weak var scrollView: UIScrollView?
    @ObservationIgnored private lazy var pan: UIPanGestureRecognizer = {
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.delegate = self
        pan.maximumNumberOfTouches = 1
        return pan
    }()

    func attach(_ scrollView: UIScrollView) {
        guard scrollView !== self.scrollView else { return }
        self.scrollView?.removeGestureRecognizer(pan)
        self.scrollView = scrollView
        scrollView.addGestureRecognizer(pan)
        // 两个拖动识别器挂在同一个视图上，谁先认出来不一定：让列表的滚动等这边先表态。
        // 不是向左的横拖时这边在 shouldBegin 里立刻放弃，滚动不受拖累。
        scrollView.panGestureRecognizer.require(toFail: pan)
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        let velocity = pan.velocity(in: scrollView)
        return velocity.x < 0 && abs(velocity.x) > abs(velocity.y)
    }

    @objc private func panned(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began, .changed:
            offset = min(WontonTheme.gesture.timeReveal, max(0, -pan.translation(in: scrollView).x))
        default:
            let reduceMotion = UIAccessibility.isReduceMotionEnabled
            withAnimation(reduceMotion ? .easeOut(duration: WontonTheme.motion.reduced) : .spring(response: 0.25)) {
                offset = 0
            }
        }
    }
}

/// 一条消息跟着左滑露出时间：自己的气泡往左让开，时间从右边滑进来。
struct RevealedTime: ViewModifier {
    let timestampMs: UInt64
    let isOwn: Bool
    let reveal: TimeReveal?
    @Environment(\.wontonColors) private var colors

    func body(content: Content) -> some View {
        let offset = reveal?.offset ?? 0
        let width = WontonTheme.gesture.timeReveal
        content
            .frame(maxWidth: .infinity, alignment: .trailing)
            // 别人的气泡只让出页边那一点：右边留的空加上它，正好放得下时间，气泡不会被推出屏幕。
            .offset(x: isOwn ? -offset : -offset * WontonTheme.spacing.pageInset / width)
            .overlay(alignment: .trailing) {
                if offset > 0 {
                    Text(TimelineRow.time(timestampMs))
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                        .fixedSize()
                        .opacity(Double(offset / width))
                        .offset(x: width - offset)
                }
            }
    }
}
