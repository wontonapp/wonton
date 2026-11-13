import SwiftUI
import UIKit
import WebKit

extension WontonGesture {
    static func commits(distance: CGFloat, velocity: CGFloat, threshold: CGFloat) -> Bool {
        distance >= threshold || (distance >= WontonTheme.gesture.flickDistance
            && velocity >= WontonTheme.gesture.flickVelocity)
    }
}

/// 面板从哪一边出来：主聊天页在上（列表到顶后往下拉），房间在下（最新一条之后往上推）。
enum PanelEdge { case top, bottom }

/// 聊天列表到顶后手指继续往下拉：距离交给面板，滚动位置钉在那一头。
struct ChatPull {
    let enabled: Bool
    let changed: (CGFloat) -> Void
    let ended: (CGFloat, CGFloat) -> Void
    let cancelled: () -> Void
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
        anchor.cancelled = pull.cancelled
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
        anchor.cancelled = pull?.cancelled
    }
}

/// 滚动视图到顶后手指继续往下拉（`edge` 为 `.bottom` 时是到底后继续往上推）：把滚动位置钉在那一头，
/// 拉的距离交出去。只认手指拖动：惯性滚到头冲出去的不经过拖动手势，不算。
final class PullAnchor: UIView {
    var enabled = true
    var changed: ((CGFloat) -> Void)?
    var ended: ((CGFloat, CGFloat) -> Void)?
    var cancelled: (() -> Void)?
    private let attachesToAncestor: Bool
    var edge: PanelEdge
    /// 面板网页才有：只认从网页露出的那条边附近起手的拖动。聊天列表整片都能拉，为空。
    var startZone: PagePullZone?
    private weak var scrollView: UIScrollView?
    private var pullStart: CGFloat?
    private var startsInZone = true

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
        if pullStart != nil {
            cancelled?()
            pullStart = nil
        }
        scrollView?.panGestureRecognizer.removeTarget(self, action: #selector(panned(_:)))
        scrollView = target
        target?.panGestureRecognizer.addTarget(self, action: #selector(panned(_:)))
    }

    @objc func panned(_ pan: UIPanGestureRecognizer) {
        guard let scrollView else { return }
        // 往打开的方向是正：到顶时往下，到底时往上。
        let sign: CGFloat = edge == .top ? 1 : -1
        let end = edge == .top ? -scrollView.adjustedContentInset.top : scrollView.bottomOffset
        let atEnd = edge == .top
            ? scrollView.contentOffset.y <= end + 0.5
            : scrollView.contentOffset.y >= end - 0.5
        let translation = sign * pan.translation(in: scrollView.superview).y
        switch pan.state {
        case .began:
            startsInZone = startZone?.contains(pan, over: scrollView, edge: edge) ?? true
            pullStart = startsInZone && atEnd && sign * pan.velocity(in: scrollView).y > 0 ? 0 : nil
        case .changed:
            guard enabled else {
                if pullStart != nil { cancelled?() }
                pullStart = nil
                return
            }
            if pullStart == nil, startsInZone, atEnd, sign * pan.velocity(in: scrollView).y > 0 {
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
        case .ended:
            if let start = pullStart {
                if enabled {
                    scrollView.contentOffset.y = end
                    ended?(translation - start, sign * pan.velocity(in: scrollView.superview).y)
                } else {
                    cancelled?()
                }
            }
            pullStart = nil
        case .cancelled, .failed:
            if pullStart != nil { cancelled?() }
            pullStart = nil
        default:
            break
        }
    }
}

/// 面板网页上能拖动面板的起手区（ADR-0052）：网页露出的那条边往里 `gesture.pageEdgeZone`。
///
/// 网页的滚动位置说明不了手指下面是什么：全屏的固定浮层（音乐播放器）、自己滚动的容器都让主滚动视图一直停在头上，
/// 只凭“滚到头了”就接手，页面里任何一处的拖动都成了拖面板（Navidrome 播放器，2026-11-12 盲测）。问页面手指下面是
/// 什么要等一次异步的脚本往返，拖动已经开始了；公网网页也不注入脚本。所以按起手的位置分：边上归面板，里面归网页。
struct PagePullZone: Equatable {
    /// 网页框被面板挡住或在屏幕外的上下两条，同 `TabWebView` 的 `obscured`。
    let obscured: UIEdgeInsets

    /// `start` 和 `visible` 在同一个坐标系里；`visible` 是网页看得见、点得到的那一块。
    static func contains(_ start: CGPoint, visible: CGRect, edge: PanelEdge) -> Bool {
        switch edge {
        case .top: start.y <= visible.minY + WontonTheme.gesture.pageEdgeZone
        case .bottom: start.y >= visible.maxY - WontonTheme.gesture.pageEdgeZone
        }
    }

    /// 拖动手势认出来时手指已经走了一小段：按起手的位置算，不按认出来的那一下。
    @MainActor
    func contains(_ pan: UIPanGestureRecognizer, over scrollView: UIScrollView, edge: PanelEdge) -> Bool {
        let now = pan.location(in: nil)
        let moved = pan.translation(in: nil)
        let start = CGPoint(x: now.x - moved.x, y: now.y - moved.y)
        var visible = scrollView.convert(scrollView.bounds, to: nil).inset(by: obscured)
        // Home 条和状态栏那两段是系统手势的地方，起手区从安全区的边往里算。
        if let window = scrollView.window {
            visible = visible.intersection(window.bounds.inset(by: window.safeAreaInsets))
        }
        return Self.contains(start, visible: visible, edge: edge)
    }
}

/// 面板网页滚到头后从边上接着拉：第 1 级进第 2 级，房间的第 2 级从顶上下拉回第 1 级（ADR-0052）。
struct PanelPagePull: UIViewRepresentable {
    let enabled: Bool
    let webView: WKWebView?
    let edge: PanelEdge
    let zone: PagePullZone
    let changed: (CGFloat) -> Void
    let ended: (CGFloat, CGFloat) -> Void
    let cancelled: () -> Void

    func makeUIView(context: Context) -> PullAnchor {
        PullAnchor(attachesToAncestor: false, edge: edge)
    }

    func updateUIView(_ anchor: PullAnchor, context: Context) {
        anchor.edge = edge
        anchor.startZone = zone
        anchor.enabled = enabled
        anchor.changed = changed
        anchor.ended = ended
        anchor.cancelled = cancelled
        anchor.attach(to: webView?.scrollView)
    }

    static func dismantleUIView(_ anchor: PullAnchor, coordinator: ()) { anchor.attach(to: nil) }
}
