import SwiftUI
import UIKit
import WontonClient

/// 水平手势先判方向才接管，垂直滚动、边缘返回和没有滑动的链接/长按仍交给系统。
struct ReplySwipe: ViewModifier {
    let quote: QuotedMessage?
    let reply: (QuotedMessage) -> Void
    @State private var offset: CGFloat = 0
    @State private var gaveFeedback = false
    @Environment(\.wontonColors) private var colors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let threshold: CGFloat = 40
    private let maximum: CGFloat = 72

    func body(content: Content) -> some View {
        content
            .offset(x: offset)
            .background(alignment: .leading) {
                WontonIcon.reply.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
                    .opacity(Double(min(offset / threshold, 1)))
                    .padding(.leading, WontonTheme.spacing.s8)
                    .accessibilityHidden(true)
            }
            .background(ReplyPanDetector(enabled: quote != nil, changed: changed, ended: ended))
    }

    private func changed(_ distance: CGFloat) {
        offset = min(maximum, max(0, distance))
        if offset >= threshold, !gaveFeedback {
            gaveFeedback = true
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        }
    }

    private func ended(_ cancelled: Bool) {
        let triggered = !cancelled && offset >= threshold
        gaveFeedback = false
        withAnimation(reduceMotion ? .easeOut(duration: WontonTheme.motion.reduced) : .spring(response: 0.25)) {
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
        // 屏幕边缘留给系统返回；不让右滑识别器抢竖向滚动或向左的手势。
        return pan.location(in: window).x > 24 && velocity.x > abs(velocity.y)
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
