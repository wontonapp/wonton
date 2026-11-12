import SwiftUI
import WebKit

/// 看手指在网页上往哪拖，不看页面滚没滚：整页滚动、页面里的容器滚动、不能滚的单页和 canvas 都一样。
/// 往下拖（往回看）叫出胶囊，往上拖（往下读）藏起来。只旁听，不拦触摸。
///
/// 手势挂在窗口上、只收落在这个网页里的触摸，不挂在 WKWebView 上：挂在网页里的手势归 WebKit 的触摸延迟手势管，
/// 页面自己处理了触摸（touchstart 里 preventDefault）就被判失败，胶囊叫不出来，全屏页就没有出口。
struct FullscreenCapsuleReveal: UIViewRepresentable {
    let enabled: Bool
    let webView: WKWebView?
    let reveal: (Bool) -> Void

    func makeUIView(context: Context) -> CapsuleRevealAnchor { CapsuleRevealAnchor() }

    func updateUIView(_ anchor: CapsuleRevealAnchor, context: Context) {
        anchor.reveal = reveal
        anchor.attach(to: webView)
        anchor.pan.isEnabled = enabled
    }

    static func dismantleUIView(_ anchor: CapsuleRevealAnchor, coordinator: ()) { anchor.detach() }
}

final class CapsuleRevealAnchor: UIView, UIGestureRecognizerDelegate {
    /// 同一方向连续拖过这么远才算，手指的抖动不算。
    private static let threshold: CGFloat = 24
    let pan = UIPanGestureRecognizer()
    var reveal: ((Bool) -> Void)?
    private weak var host: WKWebView?
    private var lastY: CGFloat = 0
    private var run: CGFloat = 0
    private var sent: Bool?

    init() {
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        pan.cancelsTouchesInView = false
        pan.delaysTouchesBegan = false
        pan.delaysTouchesEnded = false
        pan.delegate = self
        pan.addTarget(self, action: #selector(panned(_:)))
    }

    required init?(coder: NSCoder) { nil }

    func attach(to webView: WKWebView?) { host = webView }

    func detach() {
        pan.view?.removeGestureRecognizer(pan)
        host = nil
        reveal = nil
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        pan.view?.removeGestureRecognizer(pan)
        window?.addGestureRecognizer(pan)
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard let host, let view = touch.view else { return false }
        return view.isDescendant(of: host)
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool { true }

    @objc private func panned(_ recognizer: UIPanGestureRecognizer) {
        switch recognizer.state {
        case .began:
            lastY = 0; run = 0; sent = nil
        case .changed:
            let offset = recognizer.translation(in: window).y
            let step = offset - lastY
            lastY = offset
            if step == 0 { return }
            if (step > 0) != (run > 0) { run = 0 }
            run += step
            let down = run > 0
            if abs(run) >= Self.threshold, sent != down {
                sent = down
                reveal?(down)
            }
        default:
            break
        }
    }
}
