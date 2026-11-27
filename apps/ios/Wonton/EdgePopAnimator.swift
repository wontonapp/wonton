import UIKit

/// 左边缘拖动返回时的转场动画，照系统返回的样子：上面一页跟着手指往右走、左边带一道阴影，
/// 下面一页从左边三成处跟进来、先压暗一点再亮起来。只给 `EdgeBack` 拖动时用，进度由它驱动。
///
/// 上面一页拖的是它的快照，本身原地不动、只是先藏起来：SwiftUI 在转场途中按视图当时的位置记下辅助功能里的
/// 坐标，弹回以后不再更新。直接挪这一页（frame、transform 都试过）的话，拖到一半弹回后读屏和界面测试拿到的
/// 位置还在右边一屏外（iOS 26.5 模拟器实测）。只在图层上加动画能避开，但图层动画不跟交互进度停在半路。
/// 代价是拖动的这一会儿上面一页是静止的画面。
final class EdgePopAnimator: NSObject, UIViewControllerAnimatedTransitioning {
    private var animator: UIViewPropertyAnimator?

    func transitionDuration(using context: UIViewControllerContextTransitioning?) -> TimeInterval { 0.35 }

    func animateTransition(using context: UIViewControllerContextTransitioning) {
        interruptibleAnimator(using: context).startAnimation()
    }

    /// 交给 `UIPercentDrivenInteractiveTransition`：拖动时按进度停在中间，松手后从当前位置走完或退回。
    func interruptibleAnimator(using context: UIViewControllerContextTransitioning) -> UIViewImplicitlyAnimating {
        if let animator { return animator }
        let animator = makeAnimator(context)
        self.animator = animator
        return animator
    }

    func animationEnded(_ transitionCompleted: Bool) {
        animator = nil
    }

    private func makeAnimator(_ context: UIViewControllerContextTransitioning) -> UIViewPropertyAnimator {
        let animator = UIViewPropertyAnimator(duration: transitionDuration(using: context), curve: .linear)
        guard let from = context.view(forKey: .from), let under = context.view(forKey: .to),
              let toController = context.viewController(forKey: .to) else {
            animator.addAnimations {}
            animator.addCompletion { _ in context.completeTransition(!context.transitionWasCancelled) }
            return animator
        }
        let container = context.containerView
        let width = container.bounds.width
        let toFrame = context.finalFrame(for: toController)
        let moving = from.snapshotView(afterScreenUpdates: false) ?? UIView()
        moving.frame = from.frame
        // 下面一页透明（插件页层、“全部应用”层的根）时，压暗的是透出来的聊天，和系统一样。
        let dim = UIView(frame: container.bounds)
        dim.backgroundColor = .black
        dim.alpha = 0.1
        dim.isUserInteractionEnabled = false
        let shadow = Self.shadow(frame: from.frame)
        container.insertSubview(under, belowSubview: from)
        container.insertSubview(dim, belowSubview: from)
        container.insertSubview(shadow, belowSubview: from)
        container.insertSubview(moving, aboveSubview: from)
        from.layer.opacity = 0
        under.frame = toFrame.offsetBy(dx: -width * 0.3, dy: 0)

        animator.addAnimations {
            moving.frame = moving.frame.offsetBy(dx: width, dy: 0)
            shadow.frame = moving.frame
            under.frame = toFrame
            dim.alpha = 0
        }
        animator.addCompletion { _ in
            moving.removeFromSuperview()
            dim.removeFromSuperview()
            shadow.removeFromSuperview()
            from.layer.opacity = 1
            let cancelled = context.transitionWasCancelled
            if cancelled { under.removeFromSuperview() }
            context.completeTransition(!cancelled)
        }
        return animator
    }

    private static func shadow(frame: CGRect) -> UIView {
        let shadow = UIView(frame: frame)
        shadow.isUserInteractionEnabled = false
        shadow.layer.shadowColor = UIColor.black.cgColor
        shadow.layer.shadowOpacity = 0.25
        shadow.layer.shadowRadius = 8
        shadow.layer.shadowOffset = CGSize(width: -2, height: 0)
        shadow.layer.shadowPath = UIBezierPath(rect: shadow.bounds).cgPath
        return shadow
    }
}
