import SwiftUI
import WebKit

/// 看手指在网页上往哪拖，不看页面滚没滚：整页滚动、页面里的容器滚动、不能滚的单页和 canvas 都一样。
/// 往下拖（往回看）叫出胶囊，往上拖（往下读）藏起来。只旁听，不拦触摸。
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

    static func dismantleUIView(_ anchor: CapsuleRevealAnchor, coordinator: ()) { anchor.attach(to: nil) }
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

    func attach(to webView: WKWebView?) {
        guard webView !== host else { return }
        host?.removeGestureRecognizer(pan)
        host = webView
        webView?.addGestureRecognizer(pan)
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
            let offset = recognizer.translation(in: recognizer.view).y
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
