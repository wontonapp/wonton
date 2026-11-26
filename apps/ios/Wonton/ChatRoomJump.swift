import SwiftUI
import WontonClient

/// 跳到原消息在壳这边的状态（ADR-0109 第 4 节）；定位、返回栈都在核心。
struct TimelineJumpState {
    /// 点的是哪一条的引用：核心往前翻页时在它上面画加载。
    var from: String?
    /// 刚跳到、要闪一下的那一行，和这是第几次跳。
    var flash: (itemId: String, revision: Int)?
    /// 闪过几次、最后闪的是哪一行：只在走查的读数里，闪光很快淡掉，测试按它数有没有闪过。
    var flashes = 0
    var lastFlash: String?
    /// 原消息找不到的次数：每多一次，轻提示再出一回。
    var missing = 0
    var showsMissing = false
}

@MainActor
extension ChatRoomView {
    /// 核心说滚到哪条就把哪条摆到可视区中部；跳到原消息时那一行闪一下，闪完把记号清掉（不然这一行滚出去再回来会再闪）。
    func focus(_ scroller: ScrollViewProxy) {
        guard let focus = timeline.focus else { return }
        let revision = timeline.focusRevision
        jump.from = nil
        pin.unpin()
        settle(focus.itemId, at: .center, scroller, corrections: Self.focusCorrections) {
            guard focus.highlight, revision == timeline.focusRevision else { return }
            jump.flash = (focus.itemId, revision)
            jump.flashes += 1
            jump.lastFlash = focus.itemId
            let lit = WontonTheme.motion.jumpHighlight + WontonTheme.motion.panel
            DispatchQueue.main.asyncAfter(deadline: .now() + lit) {
                if jump.flash?.revision == revision { jump.flash = nil }
            }
        }
    }

    /// 离得远的行还没量过高度，第一次滚到的位置按估算的行高算、会偏：等这一轮排完再对几次。
    /// 滚之前先看这条还在不在：`scrollTo` 找不到条目会越界崩掉（见 BottomPin）。
    /// 这几下本身就是滚动，途中出浮动日期（ADR-0109 第 4 节）。主人自己滚走了（贴底了，或者在拖）就不再对，
    /// 但跳转到此结束，闪光照样在原消息上给一下。
    func settle(
        _ itemId: String, at anchor: UnitPoint, _ scroller: ScrollViewProxy, corrections: Int,
        after delay: TimeInterval = 0, done: @escaping () -> Void
    ) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            guard timeline.items.contains(where: { $0.id == itemId }) else { return done() }
            guard !pin.isPinned, !pin.isTouched else { return done() }
            pin.noteProgrammaticScroll()
            scroller.scrollTo(itemId, anchor: anchor)
            guard corrections > 0 else { return done() }
            settle(
                itemId, at: anchor, scroller, corrections: corrections - 1, after: WontonTheme.motion.feedback,
                done: done
            )
        }
    }

    /// 滚到一条之后再对几次位置。跳转的这一串比落点长：主人点完引用看一眼再碰列表（约 1 秒多）也在纠正里，
    /// 手一碰就停，闪光当场给；太短的话跳已经结束，碰不碰都一样了。
    static var focusCorrections: Int { 20 }

    /// 摆进房间的落点只对两下：落地后主人马上要能自己滚。
    static var placeCorrections: Int { 2 }
}
