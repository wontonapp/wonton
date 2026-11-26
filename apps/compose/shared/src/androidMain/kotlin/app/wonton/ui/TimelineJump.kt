package app.wonton.ui

import app.wonton.protocol.Action
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget

/**
 * 跳到引用的那条原消息。正在跳的途中再点不派第二次；落到原消息才停。
 * 先把本次跳转的序号报给核心，同一条原消息连点两下，核心也当成新的一次。`fromId` 是“回来”的落脚点：
 * 调方给 [`bottomMostItemId`]——跳之前最靠下的那条，不是目的地自己（ADR-0109）；列表空时退到最后一条。
 */
internal fun TimelineState.jumpTo(
    eventId: String,
    fromId: String,
    target: TimelineTarget,
    onAction: (Action) -> Unit,
) {
    if (jumpQuote != null) return
    jumpQuote = eventId
    jumpSeq++
    onAction(Action.Timeline(TimelineAction.RegisterFocusRevision(target, jumpSeq.toUInt())))
    onAction(Action.Timeline(TimelineAction.JumpToMessage(target, eventId, fromId)))
}

/**
 * “回来”要落到的那条：眼下露出、最靠下有内容的条目。倒排列表里下标越小越靠下；空白 AgentRun 不算内容。
 * 列表还没摆出来时退回最新的一条。
 */
internal fun TimelineState.bottomMostItemId(): String =
    list.layoutInfo.visibleItemsInfo.asSequence()
        .sortedBy { it.index }
        // 列表的键就是条目 ID：待发消息、翻页转圈也占下标，按 ID 对回来才不会错位。
        .mapNotNull { info -> items.firstOrNull { it.id == info.key }?.takeUnless { it.isBlankAgentRun }?.id }
        .firstOrNull()
        ?: items.lastOrNull { !it.isBlankAgentRun }?.id
        ?: items.lastOrNull()?.id
        .orEmpty()
