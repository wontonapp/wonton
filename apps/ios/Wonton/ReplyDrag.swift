import SwiftUI
import UIKit
import WontonClient

/// 左滑回复（ADR-0109 第 5 节）：气泡跟手往左，右侧露出随距离放大的回复箭头；拖过阈值的那一刻震一次，
/// 松手时过了阈值就进入回复状态，不管过没过都弹回原位。
///
/// 水平方向占优才接管，竖向滚动、长按菜单和链接照旧；从屏幕左边缘起手的留给系统返回。
struct ReplyDrag: ViewModifier {
    /// 核心随菜单给的引用；没有（这条不能回复）就不认拖动。
    let quote: QuotedMessage?
    let reply: (QuotedMessage) -> Void
    @State private var offset: CGFloat = 0
    @State private var gaveFeedback = false
    @Environment(\.wontonColors) private var colors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var progress: CGFloat { min(-offset / WontonTheme.gesture.replyTrigger, 1) }

    func body(content: Content) -> some View {
        content
            .offset(x: offset)
            .background(alignment: .trailing) {
                WontonIcon.reply.view(size: WontonTheme.size.icon)
                    .foregroundStyle(Color(colors.textSecondary))
                    .scaleEffect(progress)
                    .opacity(Double(progress))
                    .accessibilityHidden(true)
            }
            .background(ReplyPanDetector(enabled: quote != nil, changed: changed, ended: ended))
    }

    private func changed(_ distance: CGFloat) {
        offset = max(-WontonTheme.gesture.replyMax, min(0, distance))
        // 一次拖动只震一次：拖回阈值内再拖过去不重复。
        if -offset >= WontonTheme.gesture.replyTrigger, !gaveFeedback {
            gaveFeedback = true
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        }
    }

    private func ended(_ cancelled: Bool) {
        let triggered = !cancelled && -offset >= WontonTheme.gesture.replyTrigger
        gaveFeedback = false
        withAnimation(
            reduceMotion ? .easeOut(duration: WontonTheme.motion.reduced) : .spring(duration: WontonTheme.motion.panel)
        ) {
            offset = 0
        }
        if triggered, let quote { reply(quote) }
    }
}

private struct ReplyPanDetector: UIViewRepresentable {
    let enabled: Bool
    let changed: (CGFloat) -> Void
    let ended: (Bool) -> Void

    func makeUIView(context: Context) -> ReplyPanProbe { ReplyPanProbe() }

    func updateUIView(_ probe: ReplyPanProbe, context: Context) {
        probe.pan.isEnabled = enabled
        probe.changed = changed
        probe.ended = ended
    }

    static func dismantleUIView(_ probe: ReplyPanProbe, coordinator: ()) { probe.detach() }
}

/// 识别器挂在这一行的列表格子上：挂在 SwiftUI 的视图上会吃掉气泡里的点按和长按。
private final class ReplyPanProbe: UIView, UIGestureRecognizerDelegate {
    var changed: ((CGFloat) -> Void)?
    var ended: ((Bool) -> Void)?
    lazy var pan: UIPanGestureRecognizer = {
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.delegate = self
        pan.maximumNumberOfTouches = 1
        return pan
    }()
    private weak var row: UIView?

    init() {
        super.init(frame: .zero)
        isUserInteractionEnabled = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        var ancestor = superview
        while let view = ancestor {
            if view is UICollectionViewCell || view is UITableViewCell {
                guard view !== row else { return }
                detach()
                row = view
                view.addGestureRecognizer(pan)
                return
            }
            ancestor = view.superview
        }
    }

    func detach() {
        row?.removeGestureRecognizer(pan)
        row = nil
    }

    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        let velocity = pan.velocity(in: row)
        return pan.location(in: window).x > WontonTheme.gesture.edgeWidth
            && velocity.x < 0 && -velocity.x > abs(velocity.y)
    }

    @objc private func panned(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began, .changed: changed?(pan.translation(in: row).x)
        case .ended: ended?(false)
        case .cancelled, .failed: ended?(true)
        default: break
        }
    }
}
