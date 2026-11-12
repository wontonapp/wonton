import SwiftUI
import UIKit
import WebKit

/// 面板从哪一边出来：主聊天页在上（列表到顶后往下拉），房间在下（最新一条之后往上推）。
enum PanelEdge { case top, bottom }

/// 聊天列表到顶后手指继续往下拉：距离交给面板，滚动位置钉在那一头。
struct ChatPull {
    let enabled: Bool
    let changed: (CGFloat) -> Void
    let ended: (CGFloat, CGFloat) -> Void
}

extension EnvironmentValues {
    @Entry var chatPull: ChatPull?
}

/// 面板拉开了（或正在拉）：主界面的左边缘手势（侧边栏）要让开。
struct ChatPanelOpenKey: PreferenceKey {
    static var defaultValue: Bool { false }
    static func reduce(value: inout Bool, nextValue: () -> Bool) { value = value || nextValue() }
}

/// 放在聊天列表的内容里，找到外面的 `UIScrollView`，旁听它的拖动。
/// iOS 17 没有读滚动位置的 SwiftUI 接口（`onScrollGeometryChange` 从 iOS 18 起）。
struct ChatPullDetector: UIViewRepresentable {
    let pull: ChatPull

    func makeUIView(context: Context) -> PullAnchor { PullAnchor() }

    func updateUIView(_ anchor: PullAnchor, context: Context) {
        anchor.enabled = pull.enabled
        anchor.changed = pull.changed
        anchor.ended = pull.ended
    }
}

/// 房间的消息列表不是在内容里找外面的滚动视图（行会被回收），由 `BottomPin` 拿到它后交给这里。
struct RoomPullBinder: UIViewRepresentable {
    let anchor: PullAnchor
    let pull: ChatPull?

    func makeUIView(context: Context) -> UIView { UIView(frame: .zero) }

    func updateUIView(_ view: UIView, context: Context) {
        anchor.enabled = pull?.enabled ?? false
        anchor.changed = pull?.changed
        anchor.ended = pull?.ended
    }
}

/// 滚动视图到顶后手指继续往下拉（`edge` 为 `.bottom` 时是到底后继续往上推）：把滚动位置钉在那一头，
/// 拉的距离交出去。只认手指拖动：惯性滚到头冲出去的不经过拖动手势，不算。
final class PullAnchor: UIView {
    var enabled = true
    var changed: ((CGFloat) -> Void)?
    var ended: ((CGFloat, CGFloat) -> Void)?
    private let attachesToAncestor: Bool
    var edge: PanelEdge
    private weak var scrollView: UIScrollView?
    private var pullStart: CGFloat?

    init(attachesToAncestor: Bool = true, edge: PanelEdge = .top) {
        self.attachesToAncestor = attachesToAncestor
        self.edge = edge
        super.init(frame: .zero)
        isUserInteractionEnabled = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard attachesToAncestor else { return }
        var ancestor = superview
        while let current = ancestor, !(current is UIScrollView) { ancestor = current.superview }
        attach(to: ancestor as? UIScrollView)
    }

    func attach(to target: UIScrollView?) {
        guard target !== scrollView else { return }
        scrollView?.panGestureRecognizer.removeTarget(self, action: #selector(panned(_:)))
        scrollView = target
        target?.panGestureRecognizer.addTarget(self, action: #selector(panned(_:)))
    }

    @objc private func panned(_ pan: UIPanGestureRecognizer) {
        guard let scrollView else { return }
        // 往打开的方向是正：到顶时往下，到底时往上。
        let sign: CGFloat = edge == .top ? 1 : -1
        let end = edge == .top ? -scrollView.adjustedContentInset.top : scrollView.bottomEnd
        let atEnd = edge == .top
            ? scrollView.contentOffset.y <= end + 0.5
            : scrollView.contentOffset.y >= end - 0.5
        let translation = sign * pan.translation(in: scrollView.superview).y
        switch pan.state {
        case .began:
            pullStart = nil
        case .changed:
            guard enabled else { return }
            if pullStart == nil, atEnd, sign * pan.velocity(in: scrollView).y > 0 {
                pullStart = translation
            }
            guard let start = pullStart else { return }
            let distance = translation - start
            if distance < 0 {
                pullStart = nil
                changed?(0)
                return
            }
            scrollView.contentOffset.y = end
            changed?(distance)
        case .ended, .cancelled, .failed:
            if let start = pullStart, enabled {
                scrollView.contentOffset.y = end
                ended?(translation - start, sign * pan.velocity(in: scrollView.superview).y)
            }
            pullStart = nil
        default:
            break
        }
    }
}

private extension UIScrollView {
    /// 滚到最底下时的偏移；内容不满一屏时就是顶上。
    var bottomEnd: CGFloat {
        max(-adjustedContentInset.top, contentSize.height + adjustedContentInset.bottom - bounds.height)
    }
}

/// 第 1 级时面板网页滚到顶后继续往下拉，进第 2 级（ADR-0052）。
struct PanelPagePull: UIViewRepresentable {
    let enabled: Bool
    let webView: WKWebView?
    let edge: PanelEdge
    let changed: (CGFloat) -> Void
    let ended: (CGFloat, CGFloat) -> Void

    func makeUIView(context: Context) -> PullAnchor {
        PullAnchor(attachesToAncestor: false, edge: edge)
    }

    func updateUIView(_ anchor: PullAnchor, context: Context) {
        anchor.edge = edge
        anchor.enabled = enabled
        anchor.changed = changed
        anchor.ended = ended
        anchor.attach(to: webView?.scrollView)
    }

    static func dismantleUIView(_ anchor: PullAnchor, coordinator: ()) { anchor.attach(to: nil) }
}
