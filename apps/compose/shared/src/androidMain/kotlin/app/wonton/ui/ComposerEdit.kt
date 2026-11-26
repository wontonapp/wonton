package app.wonton.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ComposerEdit
import app.wonton.protocol.ReplyQuote
import app.wonton.protocol.TimelineAction
import app.wonton.protocol.TimelineTarget
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.WontonIcon
import app.wonton.ui.design.WontonIconButton

/** 输入框上方的编辑条（ADR-0109 第 10 节）：照回复条的样子，铅笔、“编辑消息”、原正文摘要和关闭。 */
@Composable
internal fun ComposerEditBar(edit: ComposerEdit, onCancel: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().testTag("composer-edit-bar").padding(
            start = WontonTheme.spacing.s16,
            end = WontonTheme.spacing.s8,
            bottom = WontonTheme.spacing.s4,
        ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            WontonIcon.Compose,
            // 装饰：旁边的字说明了。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconTiny),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
        Column(Modifier.weight(1f).padding(horizontal = WontonTheme.spacing.s8)) {
            Text(
                stringResource(R.string.message_edit_title),
                style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
            Text(
                edit.summary,
                style = WontonTheme.typography.caption,
                color = WontonTheme.colors.textSecondary.toColor(),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.testTag("edit-summary"),
            )
        }
        WontonIconButton(WontonIcon.Close, stringResource(R.string.common_close), onCancel)
    }
}

/** 一次发送：编辑时发 `EditMessage`，否则按附件区状态发消息或附件。 */
@Composable
internal fun rememberComposerSend(
    target: TimelineTarget,
    edit: ComposerEdit?,
    reply: ReplyQuote?,
    viaTray: Boolean,
    core: ClientCore,
    onTyping: (Boolean) -> Unit,
    onSent: () -> Unit,
    onReject: (Problem) -> Unit,
): (String) -> Unit = remember(target, edit, reply, viaTray) {
    { text ->
        if (edit != null && text.isBlank()) {
            // 确认的是正文：编辑时正文空着不算一次发送。
        } else if (edit != null) {
            // 编辑只改这一条的正文；发送成功后核心在没再打字时把原草稿恢复回来，壳等下一批换回去。
            val problem = core.send(Action.Timeline(TimelineAction.EditMessage(target, edit.eventId, text)))
            if (problem == null) onTyping(false) else onReject(problem)
        } else {
            val problem = core.send(sendAction(target, text, reply, viaTray))
            if (problem == null) {
                onTyping(false)
                onSent()
            } else {
                onReject(problem)
            }
        }
    }
}

/**
 * 退出编辑态时恢复什么：核心对“进编辑前没打过字”回 `None`，这时恢复空正文，别把编辑的正文留成草稿。
 */
fun restoredDraft(body: String?, reply: ReplyQuote?): Pair<String, ReplyQuote?> = body.orEmpty() to reply

/** 退出编辑态（取消或发成功）后把核心恢复的草稿读回来；刚进编辑态时读到的还是上一条草稿，跳过。 */
@Composable
internal fun RestoreDraftOnEditEnd(
    edit: ComposerEdit?,
    target: TimelineTarget,
    core: ClientCore,
    onRestoreDraft: (String, ReplyQuote?) -> Unit,
) {
    var wasEditing by remember { mutableStateOf(false) }
    LaunchedEffect(edit) {
        if (wasEditing && edit == null) {
            runCatching { core.draft(target) }.onSuccess { stored ->
                val (restored, restoredReply) = restoredDraft(stored?.body, stored?.reply)
                onRestoreDraft(restored, restoredReply)
            }
        }
        wasEditing = edit != null
    }
}
