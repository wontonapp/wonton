import ObjectiveC
import UIKit

/// 除了贴边几点，系统返回手势收不到触摸：模拟器上从边缘往里 8 点起手就全落到内容上（#685）。
/// 这里在左边 24 点里自己接一条平移手势，按拖动进度驱动系统自己的返回转场；系统那条一直关着，
/// 免得两条手势在不同宽度上各认一半。关掉以后 SwiftUI 换页时会把它重开，所以一开就再关回去。
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
        forwardedDelegate?.navigationController?(
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
}

/// 左边 `WontonTheme.gesture.edgeWidth` 点内的平移 = 返回。只在该起点、有上一页、没有别的转场时开始。
final class EdgeBack: NSObject, UIGestureRecognizerDelegate {

    private(set) weak var navigation: UINavigationController?
    let pan = UIPanGestureRecognizer()
    var transition = UIPercentDrivenInteractiveTransition()
    private(set) var isInteractive = false
    var waitsForContent = false
    /// 方向还没定：竖直的先走就让给滚动。
    private var decided = false
    private var gaveUp = false

    /// 拖动途中视图被摘掉时复位；不复位的话，下一次返回键的 pop 会拿到没人驱动的交互控制器。
    func resetInteractive() {
        isInteractive = false
    }

    init(navigation: UINavigationController) {
        self.navigation = navigation
        super.init()
        pan.maximumNumberOfTouches = 1
        pan.delegate = self
        pan.addTarget(self, action: #selector(drag(_:)))
    }

    func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
        guard let navigation, navigation.viewControllers.count > 1,
              navigation.transitionCoordinator?.isAnimated != true,
              !UINavigationController.keepsRoomOnSideChat else { return false }
        // shouldBegin 是过了阈值才问，现场的 location 已经在起手点右边；减去已拖距离还原落点。
        // 导航视图为盖住安全区会向左右探出去，左缘为负；屏幕上的左缘是 0，分栏时才是列自己的左边。
        let origin = max(0, navigation.view.convert(.zero, to: nil).x)
        let start = gesture.location(in: nil).x - pan.translation(in: navigation.view).x - origin
        let velocity = pan.velocity(in: navigation.view)
        NSLog("EDGE shouldBegin start=%.1f v=(%.0f,%.0f)", start, velocity.x, velocity.y)
        guard velocity.x > 0, abs(velocity.x) > abs(velocity.y) else { return false }
        return start <= WontonTheme.gesture.edgeWidth
    }

    func gestureRecognizer(
        _ gesture: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool { true }

    @objc private func drag(_ pan: UIPanGestureRecognizer) {
        guard let navigation else { return }
        let width = navigation.view.bounds.width
        let move = pan.translation(in: navigation.view)
        switch pan.state {
        case .began:
            decided = false
            gaveUp = false
            isInteractive = false
            transition = UIPercentDrivenInteractiveTransition()
        case .changed:
            if !decided {
                if abs(move.x) < 6, abs(move.y) < 6 { return }
                decided = true
                guard move.x > 0, abs(move.x) > abs(move.y) else {
                    gaveUp = true
                    return
                }
                NSLog("EDGE custom edge gesture took over x=\(move.x)")
                isInteractive = true
                _ = navigation.popViewController(animated: true)
            }
            guard isInteractive, !gaveUp else { return }
            transition.update(min(1, max(0, move.x / width)))
        case .ended, .cancelled, .failed:
            guard isInteractive, !gaveUp else { return }
            if pan.velocity(in: navigation.view).x > 300 || move.x > width * 0.4 {
                transition.finish()
            } else {
                transition.cancel()
            }
            isInteractive = false
        default:
            break
        }
    }
}
