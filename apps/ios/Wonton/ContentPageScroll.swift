import SwiftUI
import WebKit

struct ContentPageScroll: ViewModifier {
    let session: TabWebSession
    @SceneStorage("content-page-scroll-x") private var savedX = 0.0
    @SceneStorage("content-page-scroll-y") private var savedY = 0.0
    @State private var position = ContentScrollPosition()

    func body(content: Content) -> some View {
        content
            .onChange(of: session.webView, initial: true) { _, view in
                position.attach(view?.scrollView, target: CGPoint(x: savedX, y: savedY)) { point in
                    savedX = point.x
                    savedY = point.y
                }
                if session.ready { position.restore() }
            }
            .onChange(of: session.ready, initial: true) { _, ready in
                if ready { position.restore() }
            }
            .onDisappear { position.detach() }
    }
}

@MainActor
private final class ContentScrollPosition {
    private weak var scrollView: UIScrollView?
    private var observations: [NSKeyValueObservation] = []
    private var target: CGPoint?
    private var ready = false

    func attach(_ scrollView: UIScrollView?, target: CGPoint, save: @escaping (CGPoint) -> Void) {
        detach()
        guard let scrollView else { return }
        self.scrollView = scrollView
        self.target = target
        observations = [
            scrollView.observe(\.contentSize) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.restoreIfReady() }
            },
            scrollView.observe(\.bounds) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.restoreIfReady() }
            },
            scrollView.observe(\.contentOffset) { [weak self] view, _ in
                MainActor.assumeIsolated {
                    guard view.isDragging || view.isDecelerating else { return }
                    self?.target = nil
                    save(view.contentOffset)
                }
            }
        ]
    }

    func restore() {
        ready = true
        restoreIfReady()
    }

    private func restoreIfReady() {
        guard ready, let scrollView, let target,
              scrollView.bounds.width > 0, scrollView.bounds.height > 0,
              max(0, scrollView.contentSize.width - scrollView.bounds.width) >= max(0, target.x),
              max(0, scrollView.contentSize.height - scrollView.bounds.height) >= max(0, target.y) else { return }
        self.target = nil
        scrollView.setContentOffset(target, animated: false)
    }

    func detach() {
        observations.removeAll()
        scrollView = nil
        target = nil
        ready = false
    }
}
