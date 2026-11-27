import SwiftUI
import UIKit

/// 让全屏盖子看起来和推进导航栈的二级页一样：从右边推进来，左边缘右滑跟手滑走，关闭按钮也是往右滑走。
/// 只给 `edgeDismisses` 标过的盖子用（旁聊，ADR-0093）。画面和返回共用 `EdgePopScene`，拖动的判定和返回共用
/// `EdgeBack`。
///
/// 盖子的呈现和收起都不带 SwiftUI 的动画（`fullScreenCover` 只会从下往上），动画全在这里自己画：
/// - 推进来：盖之前先拍下整屏（`pendingUnder`），盖子一出现就把它垫在下面、倒着放一遍滑走。
/// - 滑走：不让 UIKit 开始收盖子，而是把盖子下面那页临时放进盖子所在的容器里按进度画。SwiftUI 一看到盖子开始收
///   就清掉自己的呈现状态、把盖子里的内容抽走，拖一半弹回时剩下一张空白页（iOS 26.5 模拟器实测）。滑完了才让
///   SwiftUI 不带动画地收掉盖子，那时画面上已经只剩下面那页（`handBackUnder`）。
@MainActor
final class EdgeDismiss {
    /// 开始滑走和弹回：盖着时下面那页是藏起来的，滑的时候要露出来。
    var peek: (Bool) -> Void = { _ in }
    /// 拖完：不带动画地收掉盖子，把 SwiftUI 里的状态对上。
    var close: () -> Void = {}
    /// 下一次盖子出现时垫在下面的整屏快照：有它就从右边推进来。
    var pendingUnder: UIView?
    private weak var cover: UIViewController?
    private var scene: EdgePopScene?
    /// 这一次滑走走完以后做什么：拖的是 `close`，关闭按钮这类是 `slideOut` 给的。
    private var afterSlide: (() -> Void)?
    /// 滑完到盖子真正收掉之间挡着的那张快照（`EdgePopScene.handBackUnder`）。
    private var still: UIView?

    /// `navigation` 所在的盖子：它是盖子里最外面那层，往上没有父控制器、自己是被呈现的，下面那页拿得到。
    static func cover(of navigation: UINavigationController) -> UIViewController? {
        var top: UIViewController = navigation
        while let parent = top.parent { top = parent }
        guard top.presentingViewController?.view != nil, top.view.superview != nil,
              top.presentedViewController == nil, !top.isBeingDismissed else { return nil }
        return top
    }

    /// 正在滑（拖着、推进来或滑走的动画还没完）。
    var isSliding: Bool { scene != nil || pushing }
    private var pushing = false

    func attach(to navigation: UINavigationController) {
        navigation.setEdgeDismiss(self)
        var top: UIViewController = navigation
        while let parent = top.parent { top = parent }
        if top.presentingViewController != nil { cover = top }
    }

    /// 盖子刚出现：有快照就从右边推进来，样子是 `EdgePopScene` 倒着放。推的是盖子本身（内容照常加载），
    /// 不是快照：这时它还没画过，拍出来是空的（iOS 26.5 模拟器实测）。推进来不会半路停下，辅助功能里的位置跟得上。
    func slideIn() {
        guard let under = pendingUnder, let cover, let container = cover.view.superview else { return }
        pendingUnder = nil
        let page: UIView = cover.view
        let width = container.bounds.width
        let dim = UIView(frame: container.bounds)
        dim.backgroundColor = .black
        dim.alpha = 0
        dim.isUserInteractionEnabled = false
        let shadow = EdgePopScene.shadow(frame: page.frame)
        under.frame = container.bounds
        container.insertSubview(under, belowSubview: page)
        container.insertSubview(dim, belowSubview: page)
        container.insertSubview(shadow, belowSubview: page)
        let offscreen = CGAffineTransform(translationX: width, y: 0)
        page.transform = offscreen
        shadow.transform = offscreen
        pushing = true
        let animator = UIViewPropertyAnimator(duration: EdgePopScene.duration, timingParameters: Self.timing)
        animator.addAnimations {
            page.transform = .identity
            shadow.transform = .identity
            under.frame = container.bounds.offsetBy(dx: -width * 0.3, dy: 0)
            dim.alpha = 0.1
        }
        animator.addCompletion { [weak self] _ in
            page.transform = .identity
            [under, dim, shadow].forEach { $0.removeFromSuperview() }
            self?.pushing = false
        }
        animator.startAnimation()
    }

    /// 不是拖的关法（关闭按钮、旁聊结束）：往右滑走，走完做 `done`。盖子不在了就直接做。
    func slideOut(then done: @escaping () -> Void) {
        guard let cover, cover.presentingViewController != nil, !cover.isBeingDismissed,
              !isSliding, begin(cover) else { return done() }
        afterSlide = done
        end(finishing: true)
    }

    /// 左边缘开始拖。
    @discardableResult
    func begin(_ cover: UIViewController) -> Bool {
        guard scene == nil, let container = cover.view.superview,
              let under = cover.presentingViewController?.view else { return false }
        peek(true)
        let scene = EdgePopScene(container: container, from: cover.view, under: under, underFrame: container.bounds)
        scene.animator.pauseAnimation()
        self.scene = scene
        self.cover = cover
        afterSlide = close
        return true
    }

    func update(_ progress: CGFloat) {
        scene?.animator.fractionComplete = progress
    }

    func end(finishing: Bool) {
        guard let scene else { return }
        let animator = scene.animator
        animator.isReversed = !finishing
        animator.addCompletion { [weak self] _ in self?.ended(finishing) }
        animator.continueAnimation(withTimingParameters: Self.timing, durationFactor: 1)
    }

    private static var timing: UICubicTimingParameters { UICubicTimingParameters(animationCurve: .easeOut) }

    private func ended(_ finished: Bool) {
        guard let scene else { return }
        self.scene = nil
        let after = afterSlide
        afterSlide = nil
        // 滑的途中盖子被别处收掉了（旁聊结束、离开房间）：下面那页 UIKit 已经放回去，只拿掉自己加的东西。
        guard let cover, cover.presentingViewController != nil, !cover.isBeingDismissed else {
            scene.cleanUp(showingFrom: true)
            peek(false)
            return
        }
        scene.cleanUp(showingFrom: !finished)
        if finished {
            still = scene.handBackUnder()
            after?()
            // 盖子收掉时 `coverLeft` 会拿掉快照；万一没收掉，也别让它一直挡着。
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in self?.dropStill() }
        } else {
            scene.restoreUnder()
            peek(false)
        }
    }

    /// 盖子离开了屏幕（收掉，或者盖子里又推进了一页）：滑着的场景和挡着的快照都收掉。
    func coverLeft() {
        dropStill()
        guard let scene else { return }
        self.scene = nil
        afterSlide = nil
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
    /// 这一页在全屏盖子的导航栈里当根：盖子按二级页的样子推进来、滑走（`EdgeDismiss`）。
    func edgeDismisses(_ dismiss: EdgeDismiss) -> some View {
        background(EdgeDismissHook(dismiss: dismiss).frame(width: 0, height: 0))
    }
}

/// 挂进 SwiftUI 的一个空控制器：进了导航栈就把 `EdgeDismiss` 交给那个栈的边缘手势，盖子出现时推进来。
private struct EdgeDismissHook: UIViewControllerRepresentable {
    let dismiss: EdgeDismiss

    func makeUIViewController(context: Context) -> Hook { Hook(dismiss: dismiss) }

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

        /// 在窗口里、排好版、还没画第一帧：这时候换成推进来的画面，盖子不会先整个闪一下。
        override func viewIsAppearing(_ animated: Bool) {
            super.viewIsAppearing(animated)
            attach()
            dismiss.slideIn()
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            dismiss.coverLeft()
        }

        /// SwiftUI 不一定把它挂成子控制器，`navigationController` 取不到时顺着响应链找。
        func attach() {
            var responder: UIResponder? = view
            while let current = responder, !(current is UINavigationController) { responder = current.next }
            guard let navigation = navigationController ?? responder as? UINavigationController else { return }
            dismiss.attach(to: navigation)
        }
    }
}
