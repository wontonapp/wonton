import UIKit

/// 上面一页跟着手指往右走、左边带一道阴影，下面一页从左边三成处跟进来、先压暗一点再亮起来。
/// 返回（`EdgePopAnimator`）和旁聊盖子的滑走（`EdgeDismiss`）共用这一套视图和动画。
///
/// 上面一页拖的是它的快照，本身原地不动、只是先藏起来：SwiftUI 在转场途中按视图当时的位置记下辅助功能里的
/// 坐标，弹回以后不再更新。直接挪这一页（frame、transform 都试过）的话，拖到一半弹回后读屏和界面测试拿到的
/// 位置还在右边一屏外（iOS 26.5 模拟器实测）。只在图层上加动画能避开，但图层动画不跟交互进度停在半路。
/// 代价是拖动的这一会儿上面一页是静止的画面。
@MainActor
struct EdgePopScene {
    static let duration: TimeInterval = 0.35
    /// 从“上面一页在原位”走到“下面一页在原位”的动画；完成回调由调用的一方加。
    let animator: UIViewPropertyAnimator
    private let from: UIView
    private let added: [UIView]
    private let under: UIView
    /// 临时放 `under` 的容器。`under` 已经不在它里面（盖子被别处收掉、UIKit 把它放回了窗口）就不再动它。
    private weak var container: UIView?
    /// `under` 原来在哪：弹回时放回去。全屏盖子下面那页平时不在窗口里，`superview` 是空的。
    private let underHome: Home

    private struct Home {
        let superview: UIView?
        let index: Int
        let frame: CGRect
    }

    /// 把 `under` 放到 `from` 底下（终点 `underFrame`），再铺上压暗、阴影和 `from` 的快照。
    init(container: UIView, from: UIView, under: UIView, underFrame: CGRect) {
        let width = container.bounds.width
        let moving = from.snapshotView(afterScreenUpdates: false) ?? UIView()
        moving.frame = from.frame
        // 下面一页透明（插件页层、“全部应用”层的根）时，压暗的是透出来的聊天，和系统一样。
        let dim = UIView(frame: container.bounds)
        dim.backgroundColor = .black
        dim.alpha = 0.1
        dim.isUserInteractionEnabled = false
        let shadow = Self.shadow(frame: from.frame)
        underHome = Home(
            superview: under.superview, index: under.superview?.subviews.firstIndex(of: under) ?? 0, frame: under.frame
        )
        container.insertSubview(under, belowSubview: from)
        container.insertSubview(dim, belowSubview: from)
        container.insertSubview(shadow, belowSubview: from)
        container.insertSubview(moving, aboveSubview: from)
        from.layer.opacity = 0
        under.frame = underFrame.offsetBy(dx: -width * 0.3, dy: 0)

        animator = UIViewPropertyAnimator(duration: Self.duration, curve: .linear)
        animator.addAnimations {
            moving.frame = moving.frame.offsetBy(dx: width, dy: 0)
            shadow.frame = moving.frame
            under.frame = underFrame
            dim.alpha = 0
        }
        self.from = from
        self.under = under
        self.container = container
        added = [moving, dim, shadow]
    }

    /// 弹回以后把下面那页放回原处。
    func restoreUnder() {
        guard let container, under.superview === container else { return }
        under.frame = underHome.frame
        if let home = underHome.superview {
            home.insertSubview(under, at: min(underHome.index, home.subviews.count))
        } else {
            under.removeFromSuperview()
        }
    }

    /// 拖完关盖子时把下面那页还回原处：留在盖子的容器里会跟着容器一起被拿走，整屏黑掉（iOS 26.5 模拟器实测）。
    /// 还回去到盖子真正收掉之间，容器里垫一张它的快照挡住；返回这张快照，盖子收掉时由调用的一方拿掉。
    func handBackUnder() -> UIView? {
        guard let container, under.superview === container else { return nil }
        let still = under.snapshotView(afterScreenUpdates: false)
        if let still {
            still.frame = under.frame
            still.isUserInteractionEnabled = false
            container.addSubview(still)
        }
        restoreUnder()
        return still
    }

    /// 拿掉快照、压暗和阴影。`showingFrom` 为假时上面一页留着藏起来，等它自己被收走，免得收走前闪一下。
    func cleanUp(showingFrom: Bool) {
        added.forEach { $0.removeFromSuperview() }
        if showingFrom { from.layer.opacity = 1 }
    }

    static func shadow(frame: CGRect) -> UIView {
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
