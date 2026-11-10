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
                    guard scrollView.isDragging || scrollView.isDecelerating else { return }
                    self?.isPinned = scrollView.bottomOffset - scrollView.contentOffset.y <= Self.slack
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
        stick()
    }

    /// 摆进房间的落点（ADR-0071）：停在别处，不跟着贴底。之后主人滚回底部会经 KVO 再 pin 回来。
    func unpin() {
        isPinned = false
    }

    func stick() {
        guard isPinned, let scrollView, !scrollView.isDragging else { return }
        scrollView.contentOffset.y = scrollView.bottomOffset
    }
}

private extension UIScrollView {
    var bottomOffset: CGFloat {
        max(-adjustedContentInset.top, contentSize.height + adjustedContentInset.bottom - bounds.height)
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
