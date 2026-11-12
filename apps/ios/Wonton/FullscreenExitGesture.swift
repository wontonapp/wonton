import SwiftUI
import WebKit

struct FullscreenExitGesture: UIViewRepresentable {
    let enabled: Bool
    let webView: WKWebView?
    let exit: () -> Void

    func makeUIView(context: Context) -> FullscreenEdgeAnchor { FullscreenEdgeAnchor() }

    func updateUIView(_ anchor: FullscreenEdgeAnchor, context: Context) {
        anchor.exit = exit
        anchor.webView = webView
        anchor.gesture.isEnabled = enabled
    }

    static func dismantleUIView(_ anchor: FullscreenEdgeAnchor, coordinator: ()) { anchor.detach() }
}

final class FullscreenEdgeAnchor: UIView, UIGestureRecognizerDelegate {
    let gesture = UIScreenEdgePanGestureRecognizer()
    weak var webView: WKWebView?
    var exit: (() -> Void)?

    init() {
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        gesture.edges = .left
        gesture.delegate = self
        gesture.addTarget(self, action: #selector(panned(_:)))
    }

    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        gesture.view?.removeGestureRecognizer(gesture)
        window?.addGestureRecognizer(gesture)
    }

    func detach() {
        gesture.view?.removeGestureRecognizer(gesture)
        exit = nil
    }

    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard gestureRecognizer === gesture else { return super.gestureRecognizerShouldBegin(gestureRecognizer) }
        guard webView?.canGoBack != true else { return false }
        let velocity = gesture.velocity(in: window)
        return velocity.x > 0 && velocity.x > abs(velocity.y)
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer, shouldBeRequiredToFailBy otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        otherGestureRecognizer === webView?.scrollView.panGestureRecognizer
    }

    @objc private func panned(_ recognizer: UIScreenEdgePanGestureRecognizer) {
        guard recognizer.state == .ended, webView?.canGoBack != true, let window else { return }
        let distance = recognizer.translation(in: window).x
        let velocity = recognizer.velocity(in: window).x
        if distance > window.bounds.width / 3 || (distance > 24 && velocity > 500) { exit?() }
    }
}
