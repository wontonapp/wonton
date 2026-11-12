package app.wonton.ui

import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableIntState
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.QuotedMessage
import app.wonton.protocol.ReplyQuote
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget

/**
 * 输入框里的草稿和正在回复的消息。草稿在核心里按房间存，这里只是它的一份本地副本：退出房间、重启 App 再回来还在。
 * 每次改动都把正文和回复对象一起交给核心——不带回复对象会把存着的回复清掉。
 */
class RoomDraft(
    private val roomId: String,
    private val body: MutableState<String>,
    private val quoted: MutableState<ReplyQuote?>,
    private val focus: MutableIntState,
    private val touchedState: MutableState<Boolean>,
    private val onAction: (Action) -> Unit,
) {
    /** 主人在这间房里动过草稿。动过之后，迟到的读回（进房间时排的那次）不能把旧快照盖上来。 */
    private var touched: Boolean
        get() = touchedState.value
        set(value) {
            touchedState.value = value
        }
    val text: String
        get() = body.value

    val reply: ReplyQuote?
        get() = quoted.value

    /** 开始回复时加一，输入框按它拿回光标。 */
    var focusToken by focus

    fun change(value: String) {
        body.value = value
        touched = true
        save()
    }

    fun replyTo(message: QuotedMessage) {
        quoted.value = ReplyQuote.Ready(message)
        focusToken++
        touched = true
        save()
    }

    fun stopReplying() {
        quoted.value = null
        touched = true
        save()
    }

    /** 字和附件发出去了：草稿整个清掉，别让回复对象留下来。 */
    fun sent() {
        body.value = ""
        quoted.value = null
        touched = true
        save()
    }

    private fun save() = onAction(
        Action.Timeline(
            TimelineAction.SetDraft(TimelineTarget(roomId, null), body.value, quoted.value?.eventId),
        ),
    )
}

/**
 * 草稿和回复状态。进房间读一次核心存着的草稿；走之前已经打了字就不盖掉正文，进程被杀后回来时正文还在、
 * 回复对象没了，单独补上。写只在主人改输入框、开始或取消回复时发，别把读到的初值也写回去：
 * 房间还没同步到本机时读会失败，写一个空串会把存的草稿清掉。
 */
@Composable
fun rememberRoomDraft(core: ClientCore, roomId: String, onAction: (Action) -> Unit): RoomDraft {
    val body = rememberSaveable(roomId) { mutableStateOf("") }
    val quoted = remember(roomId) { mutableStateOf<ReplyQuote?>(null) }
    val focus = remember(roomId) { mutableIntStateOf(0) }
    val touched = remember(roomId) { mutableStateOf(false) }
    LaunchedEffect(core, roomId) {
        runCatching { core.draft(TimelineTarget(roomId, null)) }.onSuccess { stored ->
            // 读回排队时排在它前面的写入都已落盘，但它排不进主人之后的新写入：动过草稿就以本机为准。
            if (!touched.value) {
                if (body.value.isEmpty()) body.value = stored?.body.orEmpty()
                if (quoted.value == null) quoted.value = stored?.reply
            }
        }.onFailure { failure ->
            Log.w(TAG, "could not load the draft for this room", failure)
        }
    }
    return remember(roomId, core, onAction) { RoomDraft(roomId, body, quoted, focus, touched, onAction) }
}

private const val TAG = "RoomDraft"
