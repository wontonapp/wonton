package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Badge
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.SideChatCard
import app.wonton.protocol.SideChatState
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.WontonIcon

/** 旁聊状态的文字（ADR-0093 §4 的表）；聊天列表的预览在组合之外取，所以给资源 ID。 */
internal val SideChatState.textRes: Int
    get() = when (this) {
        SideChatState.OPEN -> R.string.side_chat_state_open
        SideChatState.DONE -> R.string.side_chat_state_done
        SideChatState.FAILED -> R.string.side_chat_state_failed
        SideChatState.ENDED -> R.string.side_chat_state_ended
        SideChatState.RESULT_MISSING -> R.string.side_chat_state_result_missing
        SideChatState.UNTRACKED -> R.string.side_chat_state_untracked
        SideChatState.OTHER -> R.string.side_chat_state_other
    }

@Composable
internal fun sideChatStateText(state: SideChatState): String = stringResource(state.textRes)

/** 这一轮失败了用醒目的颜色，其余是轻的说明。 */
@Composable
private fun sideChatStateColor(state: SideChatState): Color = when (state) {
    SideChatState.FAILED -> WontonTheme.colors.danger
    SideChatState.OPEN, SideChatState.DONE, SideChatState.ENDED, SideChatState.RESULT_MISSING,
    SideChatState.UNTRACKED, SideChatState.OTHER -> WontonTheme.colors.textSecondary
}.toColor()

/**
 * 主对话里的旁聊卡片：标题，下面一行“旁聊 · 3 条”和状态，有未读时右边一个角标。点了进旁聊。带标记的根占满一行，
 * 普通消息下面挂的随内容宽。
 */
@Composable
internal fun SideChatCardRow(card: SideChatCard, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    val shape = RoundedCornerShape(WontonTheme.radii.card)
    val replies = card.replyCount.toInt()
    val unread = card.unreadCount.toInt()
    Row(
        modifier
            .clip(shape)
            .background(colors.surfaceSubtle.toColor(), shape)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = spacing.s12, vertical = spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.s8),
    ) {
        Icon(
            WontonIcon.Chat,
            // 装饰：右边一行写着“旁聊”。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconSmall),
            tint = colors.textSecondary.toColor(),
        )
        Column(Modifier.weight(1f, fill = false), verticalArrangement = Arrangement.spacedBy(spacing.s4)) {
            Text(
                card.title,
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(spacing.s8)) {
                Text(
                    pluralStringResource(R.plurals.side_chat_replies, replies, replies),
                    style = WontonTheme.typography.caption,
                    color = colors.textSecondary.toColor(),
                    maxLines = 1,
                )
                Text(
                    sideChatStateText(card.state),
                    style = WontonTheme.typography.caption,
                    color = sideChatStateColor(card.state),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (unread > 0) {
            val label = pluralStringResource(R.plurals.side_chat_unread, unread, unread)
            Badge(Modifier.clearAndSetSemantics { contentDescription = label }) {
                Text(if (unread > UNREAD_MAX) "$UNREAD_MAX+" else unread.toString())
            }
        }
    }
}

/** 主对话里的旁聊结果行：一行小字“旁聊「标题」：已完成”，点了进旁聊。 */
@Composable
internal fun SideChatResultRow(title: String, state: SideChatState, onClick: () -> Unit) {
    Text(
        stringResource(R.string.side_chat_result_line, title, sideChatStateText(state)),
        Modifier.fillMaxWidth()
            .clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = WontonTheme.spacing.s4),
        style = WontonTheme.typography.caption.copy(textAlign = TextAlign.Center),
        color = sideChatStateColor(state),
    )
}

/** 旁聊页顶栏下面一行：这个旁聊的状态；AI 不回报结果时是一句轻提示。 */
@Composable
internal fun SideChatHeaderLine(state: SideChatState) {
    Text(
        sideChatStateText(state),
        Modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s4),
        style = WontonTheme.typography.caption,
        color = sideChatStateColor(state),
    )
}

/** “新建聊天”选的 AI 不支持旁聊、打开了主对话：顶上一句（ADR-0093 §7）。 */
@Composable
internal fun NoSideChatsHint() {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Icon(
            WontonIcon.Info,
            // 装饰：右边就是说明。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconTiny),
            tint = colors.textSecondary.toColor(),
        )
        Text(
            stringResource(R.string.side_chat_unsupported),
            style = WontonTheme.typography.caption,
            color = colors.textSecondary.toColor(),
        )
    }
}

/** 角标数字超过这个数就写“99+”。 */
private const val UNREAD_MAX = 99
