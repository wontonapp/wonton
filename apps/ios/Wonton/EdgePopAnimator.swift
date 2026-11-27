import UIKit

/// 左边缘拖动返回时的转场动画，照系统返回的样子（`EdgePopScene`）。只给 `EdgeBack` 拖动时用，进度由它驱动。
final class EdgePopAnimator: NSObject, UIViewControllerAnimatedTransitioning {
    private var animator: UIViewPropertyAnimator?

    func transitionDuration(using context: UIViewControllerContextTransitioning?) -> TimeInterval {
        EdgePopScene.duration
    }

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
        guard let from = context.view(forKey: .from), let under = context.view(forKey: .to),
              let toController = context.viewController(forKey: .to) else {
            let animator = UIViewPropertyAnimator(duration: EdgePopScene.duration, curve: .linear)
            animator.addAnimations {}
            animator.addCompletion { _ in context.completeTransition(!context.transitionWasCancelled) }
            return animator
        }
        let scene = EdgePopScene(
            container: context.containerView, from: from, under: under,
            underFrame: context.finalFrame(for: toController)
        )
        scene.animator.addCompletion { _ in
            let cancelled = context.transitionWasCancelled
            scene.cleanUp(showingFrom: true)
            if cancelled { under.removeFromSuperview() }
            context.completeTransition(!cancelled)
        }
        return scene.animator
    }
}
