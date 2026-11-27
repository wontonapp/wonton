import ObjectiveC
import UIKit
// 子类里改 `state` 要这个子模块。
// swiftlint:disable:next duplicate_imports
import UIKit.UIGestureRecognizerSubclass

/// 除了贴边几点，系统返回手势收不到触摸：模拟器上从边缘往里 8 点起手就全落到内容上（#685）。
/// 这里在左边 24 点里自己接一条平移手势，按拖动进度驱动返回转场；系统那条一直关着，
/// 免得两条手势在不同宽度上各认一半。关掉以后 SwiftUI 换页时会把它重开，所以一开就再关回去。
///
/// UIKit 只在 delegate 同时给了自定义转场动画时才来要交互控制器：只给交互控制器、动画用系统默认的那条，
/// 交互控制器不会被用上，`popViewController` 直接播完整段返回，手指拖不住。所以自绘边缘手势拖动时
/// 由 `EdgePopAnimator` 画返回动画；返回键、代码里的 pop 和 iOS 26 整片内容的右滑仍走系统自己的。
nonisolated(unsafe) private var edgeBackKey: UInt8 = 0
nonisolated(unsafe) private var edgeBackWatchKey: UInt8 = 0
nonisolated(unsafe) private var edgeBackDelegateKey: UInt8 = 0
private final class EdgeBackDelegateBox: NSObject {
    weak var delegate: UINavigationControllerDelegate?
}

extension UINavigationController: @retroactive UINavigationControllerDelegate {
    /// 自绘边缘手势正在拖时，交给它驱动转场；别的调用一律转给 SwiftUI 原来那个 delegate，
    /// 顶掉它会让返回键和路径同步失灵。
    public func navigationController(
        _ navigationController: UINavigationController,
        interactionControllerFor animationController: UIViewControllerAnimatedTransitioning
    ) -> UIViewControllerInteractiveTransitioning? {
        if let back = edgeBack, back.isInteractive { return back.transition }
        return forwardedDelegate?.navigationController?(
            navigationController, interactionControllerFor: animationController
        )
    }

    public func navigationController(
        _ navigationController: UINavigationController,
        animationControllerFor operation: UINavigationController.Operation,
        from fromVC: UIViewController, to toVC: UIViewController
    ) -> UIViewControllerAnimatedTransitioning? {
        if operation == .pop, let back = edgeBack, back.isInteractive { return EdgePopAnimator() }
        return forwardedDelegate?.navigationController?(
            navigationController, animationControllerFor: operation, from: fromVC, to: toVC
        )
    }

    public func navigationController(
        _ navigationController: UINavigationController, willShow viewController: UIViewController, animated: Bool
    ) {
        forwardedDelegate?.navigationController?(navigationController, willShow: viewController, animated: animated)
    }

    public func navigationController(
        _ navigationController: UINavigationController, didShow viewController: UIViewController, animated: Bool
    ) {
        forwardedDelegate?.navigationController?(navigationController, didShow: viewController, animated: animated)
    }

    public func navigationControllerSupportedInterfaceOrientations(
        _ navigationController: UINavigationController
    ) -> UIInterfaceOrientationMask {
        forwardedDelegate?.navigationControllerSupportedInterfaceOrientations?(navigationController)
            ?? navigationController.topViewController?.supportedInterfaceOrientations ?? .all
    }

    public func navigationControllerPreferredInterfaceOrientationForPresentation(
        _ navigationController: UINavigationController
    ) -> UIInterfaceOrientation {
        forwardedDelegate?.navigationControllerPreferredInterfaceOrientationForPresentation?(navigationController)
            ?? .portrait
    }

    override open func viewDidLoad() {
        super.viewDidLoad()
        installEdgeBack()
        watchEdgeBack()
    }

    override open func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        installEdgeBack()
    }

    override open func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        installEdgeBack()
    }

    private var edgeBack: EdgeBack? {
        objc_getAssociatedObject(self, &edgeBackKey) as? EdgeBack
    }

    private var forwardedDelegate: UINavigationControllerDelegate? {
        get { (objc_getAssociatedObject(self, &edgeBackDelegateKey) as? EdgeBackDelegateBox)?.delegate }
        set {
            let box = objc_getAssociatedObject(self, &edgeBackDelegateKey) as? EdgeBackDelegateBox
                ?? {
                    let created = EdgeBackDelegateBox()
                    objc_setAssociatedObject(
                        self, &edgeBackDelegateKey, created, .OBJC_ASSOCIATION_RETAIN_NONATOMIC
                    )
                    return created
                }()
            box.delegate = newValue
        }
    }

    /// 装好边缘手势，并保住“系统那条关着、整片右滑先等它”的不变量。可以反复调。
    func installEdgeBack() {
        guard let system = interactivePopGestureRecognizer else { return }
        if edgeBack == nil {
            let back = EdgeBack(navigation: self)
            view.addGestureRecognizer(back.pan)
            objc_setAssociatedObject(self, &edgeBackKey, back, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        }
        if delegate !== self {
            forwardedDelegate = delegate
            delegate = self
        }
        if system.isEnabled { system.isEnabled = false }
        if #available(iOS 26, *), let back = edgeBack,
           let content = interactiveContentPopGestureRecognizer, !back.waitsForContent {
            content.require(toFail: back.pan)
            back.waitsForContent = true
        }
    }

    /// SwiftUI 换页时会把手势重开，开回来就再关掉。
    private func watchEdgeBack() {
        guard let system = interactivePopGestureRecognizer else { return }
        let observation = system.observe(\.isEnabled, options: [.new]) { [weak self] gesture, _ in
            MainActor.assumeIsolated {
                guard let self, gesture.isEnabled else { return }
                gesture.isEnabled = false
                NSLog("EDGE system gesture re-enabled, disabled again")
                self.installEdgeBack()
            }
        }
        objc_setAssociatedObject(self, &edgeBackWatchKey, observation, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    }

    override open func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        edgeBack?.resetInteractive()
    }

    /// 这个栈在全屏盖子里：根页上的左边缘右滑关掉盖子（`EdgeDismiss`）。
    func setEdgeDismiss(_ dismiss: EdgeDismiss) {
        installEdgeBack()
        edgeBack?.dismiss = dismiss
    }
}

/// 左边 `WontonTheme.gesture.edgeWidth` 点内的平移 = 返回。只在该起点、有上一页（或在盖子的根页上能关盖子）、
/// 没有别的转场时开始；起手就在区外的触摸当场失败，等它失败的手势（iOS 26 整片内容的右滑）不会被拖慢。
final class EdgeBack: NSObject, UIGestureRecognizerDelegate {

    private(set) weak var navigation: UINavigationController?
    let pan = EdgeBackPan()
    var transition = UIPercentDrivenInteractiveTransition()
    private(set) var isInteractive = false
    var waitsForContent = false
    /// 盖子里的栈才有：根页上拖动时关盖子。
    var dismiss: EdgeDismiss?

    /// 拖动途中视图被摘掉时复位；不复位的话，下一次返回键的 pop 会拿到没人驱动的交互控制器。
    func resetInteractive() {
        if isInteractive { transition.cancel() }
        isInteractive = false
    }

    init(navigation: UINavigationController) {
        self.navigation = navigation
        super.init()
        pan.maximumNumberOfTouches = 1
        pan.delegate = self
        pan.startsBack = { [weak self] touch in self?.startsBack(at: touch) ?? false }
        pan.addTarget(self, action: #selector(drag(_:)))
    }

    /// 落点在左边缘里、有上一页、没有别的转场。导航视图为盖住安全区会向左右探出去，左缘为负；
    /// 屏幕上的左缘是 0，分栏时才是列自己的左边。
    private func startsBack(at touch: UITouch) -> Bool {
        guard let navigation, canStart(navigation) else { return false }
        let origin = max(0, navigation.view.convert(CGPoint.zero, to: nil).x)
        return touch.location(in: nil).x - origin <= WontonTheme.gesture.edgeWidth
    }

    private func canStart(_ navigation: UINavigationController) -> Bool {
        canPop(navigation) || dismissing(navigation) != nil
    }

    /// 旁聊盖着时外面房间的栈不退（`keepsRoomOnSideChat`）；盖子里自己的栈照常退。
    private func canPop(_ navigation: UINavigationController) -> Bool {
        navigation.viewControllers.count > 1
            && navigation.transitionCoordinator?.isAnimated != true
            && (dismiss != nil || !UINavigationController.keepsRoomOnSideChat)
    }

    /// 根页上、标过能关的盖子：返回要关掉的那个盖子。
    private func dismissing(_ navigation: UINavigationController) -> UIViewController? {
        guard let dismiss, !dismiss.isDragging, navigation.viewControllers.count == 1,
              navigation.transitionCoordinator == nil else { return nil }
        return EdgeDismiss.cover(of: navigation)
    }

    /// 竖着先走的让给滚动。
    func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
        guard let navigation, canStart(navigation) else { return false }
        let velocity = pan.velocity(in: navigation.view)
        return velocity.x > 0 && abs(velocity.x) > abs(velocity.y)
    }

    /// 和别的手势一起认：页面里的滚动、按钮先认出来时，这条照样能开始；开始以后再把它们取消（`cancelOthers`）。
    func gestureRecognizer(
        _ gesture: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool { true }

    @objc private func drag(_ pan: EdgeBackPan) {
        guard let navigation else { return }
        let width = max(navigation.view.bounds.width, 1)
        let move = pan.translation(in: navigation.view).x
        switch pan.state {
        case .began:
            pan.cancelOthers()
            transition = UIPercentDrivenInteractiveTransition()
            transition.timingCurve = UICubicTimingParameters(animationCurve: .easeOut)
            if canPop(navigation) {
                isInteractive = true
                _ = navigation.popViewController(animated: true)
            } else if let cover = dismissing(navigation) {
                dismiss?.begin(cover)
            }
            NSLog("EDGE began interactive=%@", String(describing: navigation.transitionCoordinator?.isInteractive))
            update(progress(move, width))
        case .changed:
            update(progress(move, width))
        case .ended:
            let velocity = pan.releaseVelocity(in: navigation.view)
            let finishes = Self.finishes(progress: progress(move, width), velocity: velocity)
            NSLog("EDGE ended progress=%.2f v=%.0f finishes=%@", progress(move, width), velocity, "\(finishes)")
            end(finishing: finishes)
        case .cancelled, .failed:
            end(finishing: false)
        default:
            break
        }
    }

    private func progress(_ move: CGFloat, _ width: CGFloat) -> CGFloat { min(1, max(0, move / width)) }

    private func update(_ progress: CGFloat) {
        if isInteractive {
            transition.update(progress)
        } else {
            dismiss?.update(progress)
        }
    }

    private func end(finishing: Bool) {
        if isInteractive {
            isInteractive = false
            if finishing { transition.finish() } else { transition.cancel() }
        } else if dismiss?.isDragging == true {
            dismiss?.end(finishing: finishing)
        }
    }

    /// 和系统返回一样：往右甩就完成、往左甩就取消，否则看拖没拖过一半。
    private static func finishes(progress: CGFloat, velocity: CGFloat) -> Bool {
        if velocity > 300 { return true }
        if velocity < -300 { return false }
        return progress > 0.5
    }
}

/// 记下起手那一次触摸：区外当场失败；开始以后用它找出同一根手指上的别的手势取消掉。
final class EdgeBackPan: UIPanGestureRecognizer {
    var startsBack: (UITouch) -> Bool = { _ in false }
    private weak var touch: UITouch?
    /// 手指最后一次真动的时间和位置：按住不动再松手时，平移手势报的还是停下前的速度，要当成零。
    private var lastMove: (time: CFTimeInterval, x: CGFloat) = (0, 0)

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        guard touch == nil, let first = touches.first else { return }
        touch = first
        if !startsBack(first) { state = .failed }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        let moved = translation(in: nil).x
        if moved != lastMove.x { lastMove = (CACurrentMediaTime(), moved) }
    }

    override func reset() {
        super.reset()
        touch = nil
        lastMove = (0, 0)
    }

    /// 松手时的横向速度；松手前停了一下就是零。
    func releaseVelocity(in view: UIView?) -> CGFloat {
        CACurrentMediaTime() - lastMove.time > 0.1 ? 0 : velocity(in: view).x
    }

    /// 返回开始了，这根手指就只拖页面：同一次触摸上页面里的按钮、行、滚动、网页手势一律取消，
    /// 拖到一半松手弹回时也不会算成点了按钮或行。只取消已经开着的手势；关一下再开是 UIKit 取消手势的公开做法。
    func cancelOthers() {
        for other in touch?.gestureRecognizers ?? [] where other !== self && other.isEnabled {
            other.isEnabled = false
            other.isEnabled = true
        }
    }
}
