import SwiftUI
import UIKit
import Observation

/// 时间线贴着底部：内容高度或可见高度一变，就按真实高度滚到最底下；主人往上翻走了就不再跟。
///
/// 直接操作 List 底下的 `UIScrollView`，不用 `ScrollViewProxy.scrollTo`：scrollTo 调用时就把 id 换算成下标，
/// 真正滚动前条目变少（回复到达时常有）就越界，整个 App 崩掉（iOS 26.5 模拟器实测）。
@MainActor
@Observable
final class BottomPin {
    /// 停在离底部这么近也算贴着，不用主人滑到分毫不差。
    private static let slack: CGFloat = 20
    /// 回到底部按钮滚动过去的时长。
    private static let animationDuration: TimeInterval = 0.3

    private weak var scrollView: UIScrollView?
    private(set) var isPinned = true
    private var observations: [NSKeyValueObservation] = []
    private var heldOffset: CGFloat?
    private var updateGeneration = 0
    private var settling = false
    /// 主人在滚（拖着或惯性滑着）；停下后再过 `motion.datePillHold` 才变回假，浮动的日期胶囊按它显隐（ADR-0109 第 4 节）。
    private(set) var isScrolling = false
    @ObservationIgnored private var scrollEnd: DispatchWorkItem?
    /// 回到底部的动画在跑：期间自己引起的位移不交给 holdAtBottom 改写，免得一跳到底。
    private var animating = false
    /// 平滑滚过几次（走查读 `anim=`）：瞬时跳的路径不长这个数。
    private(set) var scrollAnimations = 0

    var viewportHeight: CGFloat { scrollView?.bounds.height ?? 0 }
    /// 可见区底边的屏幕 y：键盘和输入栏都算进 inset 之后的位置。
    var visibleBottom: CGFloat {
        guard let view = scrollView else { return 0 }
        return view.convert(CGPoint(x: 0, y: view.bounds.maxY - view.adjustedContentInset.bottom), to: nil).y
    }
    /// 主人的手指正拖着列表，或者松手后还在滑。
    var isTouched: Bool { scrollView.map { $0.isDragging || $0.isDecelerating } ?? false }

    /// List 会为末尾追加和变高的行调整 contentOffset；翻走时保留更新前的位置，不借内容高度反推主人的手势。
    func prepareForUpdate() {
        guard !isPinned, let scrollView, !scrollView.isDragging, !scrollView.isDecelerating else { return }
        if heldOffset == nil { heldOffset = scrollView.contentOffset.y }
        updateGeneration += 1
        let generation = updateGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self, generation == self.updateGeneration else { return }
            self.restoreOffset()
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.updateGeneration else { return }
                self.restoreOffset()
                self.heldOffset = nil
            }
        }
    }

    private func restoreOffset() {
        guard let heldOffset, let scrollView, !isPinned,
              !scrollView.isDragging, !scrollView.isDecelerating else { return }
        scrollView.layoutIfNeeded()
        scrollView.contentOffset.y = min(scrollView.bottomOffset, max(-scrollView.adjustedContentInset.top, heldOffset))
    }

    func attach(_ scrollView: UIScrollView) {
        guard scrollView !== self.scrollView else { return }
        self.scrollView = scrollView
        // 新列表的内边距是干净的，补过的那笔跟着作废，下一帧按新输入框位置重算。
        insetRepair = 0
        observations = [
            scrollView.observe(\.contentSize) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.stick() }
            },
            // 只看主人的手势：跟随时自己滚动、以及内容变高时离底部变远，都不算翻走。
            scrollView.observe(\.contentOffset) { [weak self] scrollView, _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if scrollView.isDragging || scrollView.isDecelerating {
                        self.noteScroll()
                        self.heldOffset = nil
                        self.isPinned = scrollView.bottomOffset - scrollView.contentOffset.y <= Self.slack
                    } else if self.isPinned {
                        self.holdAtBottom()
                    }
                }
            }
        ]
        repairInset()
        stick()
        // 内容可能在注册观察之前就整套排好了（有缓存历史的房间一进就满）：注册后再没有 contentSize
        // 变化，上面那次 stick 量到的是注册前的高度。补一次异步 stick，等这一帧排完再贴底；落点摆放
        // 的异步滚动排在它后面，会按落点覆盖。
        DispatchQueue.main.async { self.stick() }
    }

    private func noteScroll() {
        if !isScrolling { isScrolling = true }
        scrollEnd?.cancel()
        let end = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { self?.isScrolling = false }
        }
        scrollEnd = end
        DispatchQueue.main.asyncAfter(deadline: .now() + WontonTheme.motion.datePillHold, execute: end)
    }

    /// 回到底部按钮：先掐掉惯性，再平滑滚过去，途中收到的 contentSize 变化不会被自己的位移当成翻走。
    func scrollToBottom() {
        isPinned = true
        heldOffset = nil
        guard let scrollView = scrollView else { return }
        // 先占住 `animating`，停惯性引起的那次位移不再交给 holdAtBottom 改写。
        animating = true
        stopDeceleration()
        scrollAnimations += 1
        UIView.animate(withDuration: Self.animationDuration, animations: {
            scrollView.contentOffset.y = scrollView.bottomOffset
        }, completion: { [weak self] _ in
            self?.animating = false
            self?.stick()
        })
    }

    /// 松手后还在滑时，`contentOffset` 由减速器每帧改写，会在下面盖掉 `UIView.animate`，最后停在惯性自己的落点。
    /// 按 `isDecelerating` 的说法，`setContentOffset(_:animated: false)` 落到当前位置就是停掉减速（#1092）。
    private func stopDeceleration() {
        guard let scrollView, scrollView.isDecelerating else { return }
        scrollView.setContentOffset(scrollView.contentOffset, animated: false)
    }

    /// 壳自己滚（跳到原消息、摆进房间的落点）：和主人滚一样出浮动日期，停下后照常淡出。
    func noteProgrammaticScroll() { noteScroll() }

    /// 主人发了东西：不管翻到哪，回到底部看结果。惯性滑着也当场停下，不然新消息停在视野外。
    func pin() {
        isPinned = true
        heldOffset = nil
        stopDeceleration()
        stick()
    }

    /// 摆进房间的落点（ADR-0071）：停在别处，不跟着贴底。之后主人滚回底部会经 KVO 再 pin 回来。
    func unpin() {
        isPinned = false
    }

    func stick() {
        guard !animating, let scrollView, !scrollView.isDragging, !scrollView.isDecelerating else { return }
        if isPinned {
            scrollView.contentOffset.y = scrollView.bottomOffset
        } else {
            restoreOffset()
        }
    }

    /// 输入框顶在窗口里的 y；没有输入框时是无穷大。
    private var composerTop = CGFloat.greatestFiniteMagnitude
    /// 已经补进列表下内边距的那一段。
    private var insetRepair: CGFloat = 0

    /// 键盘弹着时 `safeAreaInset` 只留下键盘自己的内边距，输入框抬起来压在列表上，最后一条就藏在它后面
    /// （iOS 26.5 模拟器实测：列表可见底停在键盘上沿，输入框上沿比它高 160）。差额当场补进列表的下内边距，
    /// 输入框缩回去再撤掉。用不含这笔修补的内边距算，否则补完下一帧又会把补的撤回去。
    func noteComposer(top: CGFloat?) {
        let next = top ?? .greatestFiniteMagnitude
        guard next != composerTop else { return }
        composerTop = next
        repairInset()
    }

    private func repairInset() {
        guard let scrollView else { return }
        let visible = scrollView.convert(
            CGPoint(x: 0, y: scrollView.bounds.maxY - scrollView.adjustedContentInset.bottom + insetRepair),
            to: nil
        ).y
        let shortfall = max(0, visible - composerTop)
        let delta = shortfall - insetRepair
        guard abs(delta) > 0.5 else { return }
        insetRepair = shortfall
        var inset = scrollView.contentInset
        inset.bottom += delta
        scrollView.contentInset = inset
        stick()
    }

    /// 走查用：贴底状态和滚动的实际位置。界面上看不见，放进无障碍树给 UI 测试读。
    var snapshot: String {
        let view = scrollView
        let visibleBottom = view == nil ? -1 : visibleBottom
        let decel = view?.isDecelerating == true ? 1 : 0
        return "pin=\(isPinned) y=\(view?.contentOffset.y ?? -1) bottom=\(view?.bottomOffset ?? -1)"
            + " visibleBottom=\(visibleBottom) anim=\(scrollAnimations) decel=\(decel)"
    }

    /// List 排完新行会自己再挪一次 contentOffset，保住原先看到的那一行；键盘弹着、新行比可见区高时新消息就停在可见区下面，
    /// 内容高度也不再变，等不到下一次 stick（iOS 26.5 模拟器实测，在 stick 之后固定等两轮再拉也赶不上它）。所以贴底时
    /// 不是主人手势带来的位移一律拉回底部；放到下一轮，不在 KVO 回调里改正在观察的属性。
    private func holdAtBottom() {
        guard !animating, !settling, let scrollView,
              scrollView.bottomOffset - scrollView.contentOffset.y > Self.slack else { return }
        settling = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.settling = false
            guard self.isPinned, !self.animating, let scrollView = self.scrollView,
                  !scrollView.isDragging, !scrollView.isDecelerating else { return }
            scrollView.contentOffset.y = scrollView.bottomOffset
        }
    }
}

extension UIScrollView {
    /// 贴底的位置；内容不满一屏时就是顶上。
    ///
    /// 可见的最下面那一格比 `contentSize` 还靠下时按格子算：行高刚量出来、`contentSize` 还没跟上的那一轮也不落在半截。
    /// 停稳以后两者一致（iOS 26.5 模拟器，3000pt 高的行，2026-11-14 实测 `contentSize`、布局的内容高度和最后一格的
    /// 底边都是 21676.67）。
    var bottomOffset: CGFloat {
        let end = (self as? UICollectionView).map { view in
            max(contentSize.height, view.visibleCells.map(\.frame.maxY).max() ?? 0)
        } ?? contentSize.height
        return max(-adjustedContentInset.top, end + adjustedContentInset.bottom - bounds.height)
    }
}

/// 放在 List 的行里，把外面的 `UIScrollView` 交出去。
struct EnclosingScrollView: UIViewRepresentable {
    let found: (UIScrollView) -> Void

    func makeUIView(context: Context) -> Probe {
        Probe(found: found)
    }

    func updateUIView(_ view: Probe, context: Context) {}

    final class Probe: UIView {
        private let found: (UIScrollView) -> Void

        init(found: @escaping (UIScrollView) -> Void) {
            self.found = found
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("Only created in code")
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            var ancestor = superview
            while let view = ancestor {
                if let scrollView = view as? UIScrollView {
                    found(scrollView)
                    return
                }
                ancestor = view.superview
            }
        }
    }
}

/// 走查用：看不见的一行，写着 `BottomPin` 的贴底状态，UI 测试按标识读它。只在 Debug 包里挂。
struct ScrollDebug: View {
    let pin: BottomPin
    let keyboard: Bool
    let overlayBottom: CGFloat
    let composerHeight: CGFloat
    let viewport: String
    /// 走查用的额外一段（订阅次数、收到的批数），跟在自述后面。
    var extra = ""
    /// 正在闪的那一行，没有为空：跳转被主人碰断时也要能走查闪光有没有发生。
    var flashItem: String?
    /// 闪过几次、最后闪的是哪一行：闪光只亮一秒多，测试按计数走、按最后一行核对位置。
    var flashes: Int
    var lastFlash: String?

    /// 测试读的那一行：贴底状态、闪光读数；`extra` 是订阅侧的补充。
    private var line: String {
        let base = "\(pin.snapshot) flash=\(flashItem ?? "-") flashes=\(flashes)"
            + " last=\(lastFlash ?? "-")"
        return extra.isEmpty ? base : "\(base) \(extra)"
    }

    var body: some View {
        // 滚动位置不是可观察的状态：只在房间重画时取一次的话，测试读到的是上一次重画时的位置。
        TimelineView(.periodic(from: .now, by: 0.25)) { _ in
            let tail = " kb=\(keyboard) ov=\(Int(overlayBottom)) ch=\(Int(composerHeight)) vp=\(viewport)"
            Text(verbatim: "\(line)\(tail)")
                .accessibilityIdentifier("roomScroll")
                .allowsHitTesting(false)
                // 小到看不见：垫在玻璃和材质底下时，再淡的字也会被透出来。
                .font(.system(size: 1))
                .lineLimit(1)
                .frame(width: 1, height: 1)
                .opacity(0.02)
        }
    }
}
