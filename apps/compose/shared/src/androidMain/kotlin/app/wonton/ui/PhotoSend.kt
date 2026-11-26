package app.wonton.ui

import android.content.Context
import android.net.Uri
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.ImageUpload
import app.wonton.protocol.TimelineTarget
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

// 相册面板选好、点发送以后的这一段（ADR-0109 第 9 节）：照片逐张处理，旁聊里直接发、主对话里放进附件区，
// 报第几张没成和原因。

/** 照片转码好了放进附件区。核心要把字节写进本机的附件区目录，放到后台。 */
internal suspend fun addImage(core: ClientCore, roomId: String, upload: ImageUpload): Problem? =
    withContext(Dispatchers.IO) {
        try {
            core.addImageAttachment(TimelineTarget(roomId, null), upload)
            null
        } catch (rejection: CoreException) {
            rejection.problem
        }
    }

/**
 * 选好的照片按选的顺序逐张处理：旁聊里直接发，主对话里放进附件区。附言和回复只跟着第一张（和 iOS 一样），
 * 某一张不行就停在那里，后面的不再试，返回它是第几张和原因。
 */
internal suspend fun sendPickedPhotos(
    context: Context,
    uris: List<Uri>,
    send: ((ImageUpload, caption: String?, replyTo: String?) -> Problem?)?,
    unreadable: String,
    tryAnother: String,
    core: ClientCore,
    roomId: String,
    caption: String,
    firstReply: String?,
    onFirstSent: () -> Unit,
): PickedFailure? {
    val note = caption.trim().takeIf { it.isNotEmpty() }
    for ((index, uri) in uris.withIndex()) {
        // 主对话里附言和回复是给最后那条 SendComposer 的，照片本身不带（`send` 为空）。
        val replyTo = if (index == 0 && send != null) firstReply else null
        val firstNote = if (index == 0) note else null
        val problem = PhotoUpload.make(context, uri).fold(
            onSuccess = { upload ->
                if (send != null) send(upload, firstNote, replyTo) else addImage(core, roomId, upload)
            },
            onFailure = { failure -> Problem(unreadable, tryAnother, failure.message) },
        )
        if (problem != null) return PickedFailure(index, problem)
        if (replyTo != null) onFirstSent()
    }
    return null
}

/** 第 [index] 张没能发出去（从 0 数）和原因。 */
internal class PickedFailure(val index: Int, val problem: Problem)

/**
 * 第几张没发出去的问题：标题带上核心给的原因（分类问题在 [Problem.titleRes]、转码失败在 [Problem.title]），
 * 下一步照核心的，核心没说才“换一张”。
 */
internal fun photoFailure(context: Context, failure: PickedFailure, tryAnother: String): Problem {
    val title = context.getString(R.string.photo_failed_at, failure.index + 1)
    val cause = failure.problem.titleRes?.let(context::getString)
        ?: failure.problem.title.takeIf { it.isNotEmpty() }
    return Problem(
        title = if (cause == null) title else context.getString(R.string.photo_failed_at_reason, title, cause),
        nextStep = failure.problem.nextStepRes?.let(context::getString)
            ?: failure.problem.nextStep
            ?: tryAnother,
        detail = failure.problem.detail,
    )
}

/**
 * 附言跟着照片发出去了：草稿没动过就整份清、收回复；等的功夫又打了字就只去掉跟着走了的那句，
 * 后补的字留着，回复对象照样收。
 */
internal fun settleCaption(
    caption: String,
    draft: String,
    onDraftChange: (String) -> Unit,
    onSent: () -> Unit,
    onReplyCancel: () -> Unit,
) {
    if (draft == caption) {
        onSent()
        return
    }
    if (draft.startsWith(caption)) onDraftChange(draft.removePrefix(caption))
    onReplyCancel()
}
