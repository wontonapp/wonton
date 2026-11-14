package app.wonton.ui

import app.wonton.protocol.MessageContent
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem

/** 时间线里相邻消息的并排规则：连着发的并成一组，同一次发送的照片贴成一摞（ADR-0094）。 */

/**
 * 和上一条 `earlier` 是同一个人连着发的：并成一组，气泡挨紧，相接的角收小。
 * 中间隔着一行字的不并：这条上方写了时间，或者上一条下面写着已送达、已读。
 */
internal fun TimelineItem.joins(earlier: TimelineItem?): Boolean {
    val mine = message
    val theirs = earlier?.message
    return mine != null && theirs != null &&
        mine.senderId == theirs.senderId && !mine.startsTimeGroup && theirs.receipt == null
}

/**
 * 这条和紧跟着的 [next] 之间能不能不留衬垫贴成一摞：同一次发送（ADR-0094 的组标记）里挨着的两张照片，
 * [next] 不带引用、这条不带说明，中间也没有隔着别的行（[next] 上方的时间、这条下面的已读）。
 */
internal fun TimelineItem.flushesWith(next: TimelineItem?): Boolean {
    val mine = message
    val theirs = next?.message
    val group = mine?.group
    val image = mine?.body as? MessageContent.Image
    return group != null && image != null && theirs != null && theirs.group?.id == group.id &&
        mine.senderId == theirs.senderId && !theirs.startsTimeGroup && mine.receipt == null &&
        theirs.reply == null && image.image.caption == null && theirs.body is MessageContent.Image
}

internal val TimelineItem.message
    get() = (content as? TimelineContent.Message)?.message
