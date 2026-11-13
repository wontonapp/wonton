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

    private weak var scrollView: UIScrollView?
    private(set) var isPinned = true
    private var observations: [NSKeyValueObservation] = []
    private var heldOffset: CGFloat?
    private var updateGeneration = 0
    private var settling = false

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
        observations = [
            scrollView.observe(\.contentSize) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.stick() }
            },
            // 只看主人的手势：跟随时自己滚动、以及内容变高时离底部变远，都不算翻走。
            scrollView.observe(\.contentOffset) { [weak self] scrollView, _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if scrollView.isDragging || scrollView.isDecelerating {
                        self.heldOffset = nil
                        self.isPinned = scrollView.bottomOffset - scrollView.contentOffset.y <= Self.slack
                    } else if self.isPinned {
                        self.holdAtBottom()
                    }
                }
            }
        ]
        stick()
        // 内容可能在注册观察之前就整套排好了（有缓存历史的房间一进就满）：注册后再没有 contentSize
        // 变化，上面那次 stick 量到的是注册前的高度。补一次异步 stick，等这一帧排完再贴底；落点摆放
        // 的异步滚动排在它后面，会按落点覆盖。
        DispatchQueue.main.async { self.stick() }
    }

    /// 主人发了东西：不管翻到哪，回到底部看结果。
    func pin() {
        isPinned = true
        heldOffset = nil
        stick()
    }

    /// 摆进房间的落点（ADR-0071）：停在别处，不跟着贴底。之后主人滚回底部会经 KVO 再 pin 回来。
    func unpin() {
        isPinned = false
    }

    func stick() {
        guard let scrollView, !scrollView.isDragging, !scrollView.isDecelerating else { return }
        if isPinned {
            scrollView.contentOffset.y = scrollView.bottomOffset
        } else {
            restoreOffset()
        }
    }

    /// 走查用：贴底状态和滚动的实际位置。界面上看不见，放进无障碍树给 UI 测试读。
    var snapshot: String {
        let view = scrollView
        let visibleBottom = view.map {
            $0.convert(CGPoint(x: 0, y: $0.bounds.maxY - $0.adjustedContentInset.bottom), to: nil).y
        } ?? -1
        return "pin=\(isPinned) y=\(view?.contentOffset.y ?? -1) bottom=\(view?.bottomOffset ?? -1)"
            + " visibleBottom=\(visibleBottom)"
    }

    /// List 排完新行会自己再挪一次 contentOffset，保住原先看到的那一行；键盘弹着、新行比可见区高时新消息就停在可见区下面，
    /// 内容高度也不再变，等不到下一次 stick（iOS 26.5 模拟器实测，在 stick 之后固定等两轮再拉也赶不上它）。所以贴底时
    /// 不是主人手势带来的位移一律拉回底部；放到下一轮，不在 KVO 回调里改正在观察的属性。
    private func holdAtBottom() {
        guard !settling, let scrollView,
              scrollView.bottomOffset - scrollView.contentOffset.y > Self.slack else { return }
        settling = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.settling = false
            guard self.isPinned, let scrollView = self.scrollView,
                  !scrollView.isDragging, !scrollView.isDecelerating else { return }
            scrollView.contentOffset.y = scrollView.bottomOffset
        }
    }
}

extension UIScrollView {
    /// 贴底的位置；内容不满一屏时就是顶上。贴底（`BottomPin`）和到底后上推（`PullAnchor`）都按这一个算：
    /// 两处各算各的，差一点就是“贴着底却推不出面板”或一推就被拽回去。
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

    var body: some View {
        // 滚动位置不是可观察的状态：只在房间重画时取一次的话，测试读到的是上一次重画时的位置。
        TimelineView(.periodic(from: .now, by: 0.25)) { _ in
            Text(verbatim: pin.snapshot)
                .accessibilityIdentifier("roomScroll")
                .allowsHitTesting(false)
                .opacity(0.02)
        }
    }
}
