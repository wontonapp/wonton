import ObjectiveC
import UIKit

/// 除了贴边几点，系统返回手势收不到触摸：模拟器上从边缘往里 8 点起手就全落到内容上（#685）。
/// 这里在左边 24 点里自己接一条平移手势，按拖动进度驱动系统自己的返回转场；系统那条一直关着，
/// 免得两条手势在不同宽度上各认一半。关掉以后 SwiftUI 换页时会把它重开，所以一开就再关回去。
nonisolated(unsafe) private var edgeBackKey: UInt8 = 0
nonisolated(unsafe) private var edgeBackWatchKey: UInt8 = 0

extension UINavigationController: @retroactive UINavigationControllerDelegate {
    /// 自绘边缘手势正在拖时，交给它驱动转场；别的返回（返回键、代码弹栈）照旧用系统动画。
    public func navigationController(
        _ navigationController: UINavigationController,
        interactionControllerFor animationController: UIViewControllerAnimatedTransitioning
    ) -> UIViewControllerInteractiveTransitioning? {
        guard let back = edgeBack, back.isInteractive else { return nil }
        return back.transition
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

    /// 装好边缘手势，并保住“系统那条关着、整片右滑先等它”的不变量。可以反复调。
    func installEdgeBack() {
        guard let system = interactivePopGestureRecognizer else { return }
        if edgeBack == nil {
            let back = EdgeBack(navigation: self)
            view.addGestureRecognizer(back.pan)
            objc_setAssociatedObject(self, &edgeBackKey, back, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
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
                NSLog("EDGE 系统手势又被打开，关回去")
                self.installEdgeBack()
            }
        }
        objc_setAssociatedObject(self, &edgeBackWatchKey, observation, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    }
}

/// 左边 24 点内的平移 = 返回。只在该起点、有上一页、没有别的转场时开始。
final class EdgeBack: NSObject, UIGestureRecognizerDelegate {
    /// 起手允许离屏幕左边多远。
    static let edge: CGFloat = 24

    private(set) weak var navigation: UINavigationController?
    let pan = UIPanGestureRecognizer()
    let transition = UIPercentDrivenInteractiveTransition()
    private(set) var isInteractive = false
    var waitsForContent = false
    /// 方向还没定：竖直的先走就让给滚动。
    private var decided = false
    private var gaveUp = false

    init(navigation: UINavigationController) {
        self.navigation = navigation
        super.init()
        pan.maximumNumberOfTouches = 1
        pan.delegate = self
        pan.addTarget(self, action: #selector(drag(_:)))
    }

    func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
        guard let navigation, navigation.viewControllers.count > 1,
              navigation.transitionCoordinator?.isAnimated != true else { return false }
        return gesture.location(in: navigation.view).x <= Self.edge
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
        case .changed:
            if !decided {
                if abs(move.x) < 6, abs(move.y) < 6 { return }
                decided = true
                guard move.x > 0, abs(move.x) > abs(move.y) else {
                    gaveUp = true
                    return
                }
                NSLog("EDGE 自绘手势接管 x=\(move.x)")
                _ = navigation.popViewController(animated: true)
                isInteractive = true
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
