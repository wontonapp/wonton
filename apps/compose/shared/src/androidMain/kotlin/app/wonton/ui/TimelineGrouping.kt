package app.wonton.ui

import app.wonton.protocol.MessageContent
import app.wonton.protocol.MessageRun
import app.wonton.protocol.TimelineContent
import app.wonton.protocol.TimelineItem

/** 时间线里相邻消息的并排规则：核心给的 `run` 决定一条消息是不是接着上一条的组（ADR-0109），同一次发送的照片贴成一摞（ADR-0094）。 */

/**
 * 和上一条 `earlier` 是同一个组：上一条是组首或组中，这一条才接在它后面，气泡挨紧，相接的角收小。
 * `OTHER` 当不并排，和 `ALONE` 一样。
 */
internal fun TimelineItem.joins(earlier: TimelineItem?): Boolean {
    val mine = message
    val theirs = earlier?.message
    val continues = mine != null && (mine.run == MessageRun.MIDDLE || mine.run == MessageRun.LAST)
    val started = theirs != null && (theirs.run == MessageRun.FIRST || theirs.run == MessageRun.MIDDLE)
    return continues && started && mine?.senderId == theirs?.senderId
}

/**
 * 这条和紧跟着的 [next] 之间能不能不留衬垫贴成一摞：同一次发送（ADR-0094 的组标记）里挨着的两张照片，
 * [next] 不带引用、这条不带说明。
 */
internal fun TimelineItem.flushesWith(next: TimelineItem?): Boolean {
    val mine = message
    val theirs = next?.message
    val group = mine?.group
    val image = mine?.body as? MessageContent.Image
    return group != null && image != null && theirs != null && theirs.group?.id == group.id &&
        mine.senderId == theirs.senderId && theirs.reply == null &&
        image.image.caption == null && theirs.body is MessageContent.Image
}

internal val TimelineItem.message
    get() = (content as? TimelineContent.Message)?.message
