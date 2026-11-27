import SwiftUI
import UIKit

/// 全屏盖子根页上的左边缘右滑：和栈里的返回同一条手势（`EdgeBack`）、同一套画面（`EdgePopScene`）、
/// 同样的松手判定，拖完关掉盖子，拖一半松手弹回。只给 `edgeDismisses` 标过的盖子用（旁聊，ADR-0093）。
///
/// 拖的时候不让 UIKit 开始收盖子，而是把盖子下面那页临时放进盖子所在的容器里、自己按进度画：SwiftUI 一看到
/// 盖子开始收就清掉自己的呈现状态、把盖子里的内容抽走，拖一半弹回时剩下一张空白页（iOS 26.5 模拟器实测）。
/// 拖完了才让 SwiftUI 不带动画地收掉盖子（`close`），那时画面上已经只剩下面那页（`handBackUnder`）。
@MainActor
final class EdgeDismiss {
    /// 拖动开始和弹回：盖着时下面那页是藏起来的，拖的时候要露出来。
    let peek: (Bool) -> Void
    /// 拖完：不带动画地收掉盖子，把 SwiftUI 里的状态对上。
    let close: () -> Void
    private var scene: EdgePopScene?
    private weak var cover: UIViewController?
    /// 拖完到盖子真正收掉之间挡着的那张快照（`EdgePopScene.handBackUnder`）。
    private var still: UIView?

    init(peek: @escaping (Bool) -> Void, close: @escaping () -> Void) {
        self.peek = peek
        self.close = close
    }

    /// `navigation` 所在的盖子：它是盖子里最外面那层，往上没有父控制器、自己是被呈现的，下面那页拿得到。
    static func cover(of navigation: UINavigationController) -> UIViewController? {
        var top: UIViewController = navigation
        while let parent = top.parent { top = parent }
        guard top.presentingViewController?.view != nil, top.view.superview != nil,
              top.presentedViewController == nil, !top.isBeingPresented, !top.isBeingDismissed else { return nil }
        return top
    }

    var isDragging: Bool { scene != nil }

    func begin(_ cover: UIViewController) {
        guard scene == nil, let container = cover.view.superview,
              let under = cover.presentingViewController?.view else { return }
        peek(true)
        let scene = EdgePopScene(container: container, from: cover.view, under: under, underFrame: container.bounds)
        scene.animator.pauseAnimation()
        self.scene = scene
        self.cover = cover
    }

    func update(_ progress: CGFloat) {
        scene?.animator.fractionComplete = progress
    }

    func end(finishing: Bool) {
        guard let scene else { return }
        let animator = scene.animator
        animator.isReversed = !finishing
        animator.addCompletion { [weak self] _ in self?.ended(finishing) }
        animator.continueAnimation(
            withTimingParameters: UICubicTimingParameters(animationCurve: .easeOut), durationFactor: 1
        )
    }

    private func ended(_ finished: Bool) {
        guard let scene else { return }
        self.scene = nil
        // 拖动途中盖子被别处收掉了（旁聊结束、离开房间）：下面那页 UIKit 已经放回去，只拿掉自己加的东西。
        guard let cover, cover.presentingViewController != nil, !cover.isBeingDismissed else {
            scene.cleanUp(showingFrom: true)
            peek(false)
            return
        }
        scene.cleanUp(showingFrom: !finished)
        if finished {
            still = scene.handBackUnder()
            close()
            // 盖子收掉时 `coverLeft` 会拿掉快照；万一没收掉，也别让它一直挡着。
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in self?.dropStill() }
        } else {
            scene.restoreUnder()
            peek(false)
        }
    }

    /// 盖子离开了屏幕（收掉，或者盖子里又推进了一页）：拖着的场景和挡着的快照都收掉。
    func coverLeft() {
        dropStill()
        guard let scene else { return }
        self.scene = nil
        scene.animator.stopAnimation(true)
        scene.cleanUp(showingFrom: true)
        scene.restoreUnder()
        peek(false)
    }

    private func dropStill() {
        still?.removeFromSuperview()
        still = nil
    }
}

extension View {
    /// 这一页在全屏盖子的导航栈里当根时，左边缘右滑跟手关掉盖子（`EdgeDismiss`）。
    func edgeDismisses(peek: @escaping (Bool) -> Void, close: @escaping () -> Void) -> some View {
        background(EdgeDismissHook(peek: peek, close: close).frame(width: 0, height: 0))
    }
}

/// 挂进 SwiftUI 的一个空控制器：进了导航栈就把 `EdgeDismiss` 交给那个栈的边缘手势。
private struct EdgeDismissHook: UIViewControllerRepresentable {
    let peek: (Bool) -> Void
    let close: () -> Void

    func makeUIViewController(context: Context) -> Hook { Hook(dismiss: EdgeDismiss(peek: peek, close: close)) }

    func updateUIViewController(_ hook: Hook, context: Context) {
        hook.attach()
    }

    final class Hook: UIViewController {
        let dismiss: EdgeDismiss

        init(dismiss: EdgeDismiss) {
            self.dismiss = dismiss
            super.init(nibName: nil, bundle: nil)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { nil }

        override func didMove(toParent parent: UIViewController?) {
            super.didMove(toParent: parent)
            attach()
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            attach()
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            dismiss.coverLeft()
        }

        /// SwiftUI 不一定把它挂成子控制器，`navigationController` 取不到时顺着响应链找。
        func attach() {
            var responder: UIResponder? = view
            while let current = responder, !(current is UINavigationController) { responder = current.next }
            (navigationController ?? responder as? UINavigationController)?.setEdgeDismiss(dismiss)
        }
    }
}
