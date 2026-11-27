package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.wonton.protocol.BotView
import app.wonton.protocol.RoomRow
import app.wonton.ui.design.SheetSendBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

/**
 * 转发先选房间：上面搜，中间挨个勾，底下写附言再发。列的是聊天列表里已经加入、能发的房间，勾几个就发给几个。
 * 附言跟着转发交给核心的 `ForwardMessage.comment`：核心在每个房间先发附言、服务器确认后才发副本，顺序由核心保证。
 */
@Composable
fun ForwardSheet(
    rooms: List<RoomRow>,
    bots: List<BotView>,
    plugins: PluginsState,
    onSend: (List<RoomRow>, String) -> Unit,
    onDismiss: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var note by remember { mutableStateOf("") }
    val picked = remember { mutableStateListOf<String>() }
    val needle = query.trim()
    val shown = remember(rooms, needle) { rooms.filter { it.name.contains(needle, ignoreCase = true) } }
    WontonSheet(onDismiss, stringResource(R.string.message_forward_title)) { close ->
        WontonSearchField(
            query,
            { query = it },
            stringResource(R.string.message_forward_search),
            Modifier.padding(horizontal = WontonTheme.spacing.s16).testTag("forward-search"),
        )
        when {
            rooms.isEmpty() -> WontonRow(stringResource(R.string.message_forward_empty))
            shown.isEmpty() -> WontonRow(stringResource(R.string.message_forward_no_match))
            else -> LazyColumn(Modifier.heightIn(max = MAX_HEIGHT)) {
                items(shown, key = { it.roomId }) { row ->
                    ForwardRow(
                        row,
                        Modifier.testTag("forward-row"),
                        bots.firstOrNull { it.userId == row.directUserId },
                        plugins,
                        picked = row.roomId in picked,
                        onToggle = { if (!picked.remove(row.roomId)) picked.add(row.roomId) },
                    )
                    WontonSeparator()
                }
            }
        }
        // 附言条自己带 imePadding，键盘弹起时跟着上来。
        SheetSendBar(
            note = note,
            onNoteChange = { note = it },
            count = picked.size,
            hint = stringResource(R.string.sheet_note_hint),
            onSend = {
                val sending = rooms.filter { it.roomId in picked }
                close { onSend(sending, note) }
            },
            sendTag = "forward-send",
        )
    }
}

/** 一个能转发到的房间：头像、名字，右边是勾。 */
@Composable
private fun ForwardRow(
    row: RoomRow,
    modifier: Modifier = Modifier,
    bot: BotView?,
    plugins: PluginsState,
    picked: Boolean,
    onToggle: () -> Unit,
) {
    val colors = WontonTheme.colors
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .selectable(selected = picked, role = Role.Checkbox, onClick = onToggle)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        RoomAvatar(row, bot, plugins)
        Text(
            row.name,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = colors.textPrimary.toColor(),
            maxLines = 1,
        )
        Box(
            Modifier.size(WontonTheme.size.iconTiny)
                .background((if (picked) colors.primary else colors.surfaceSubtle).toColor(), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            if (picked) {
                Icon(
                    WontonIcon.Check,
                    // 装饰：整行的勾选状态在语义里。
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.iconTiny),
                    tint = colors.onPrimary.toColor(),
                )
            }
        }
    }
}

private val MAX_HEIGHT = 360.dp
