package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.RoomWidget
import app.wonton.protocol.WidgetEntry
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Apps
import app.wonton.ui.icons.AppsFill
import app.wonton.ui.icons.VideoCall
import app.wonton.ui.icons.VideoCallFill
import app.wonton.ui.icons.VoiceCall
import app.wonton.ui.icons.VoiceCallFill
import app.wonton.ui.icons.WontonIcon

/** “通话中”标记底色的透明度，同 [Tag]。 */
private const val ACTIVE_FILL_ALPHA = 0.14f

/** 房间顶栏的 widget 入口和通话状态（ADR-0096）：核心每一批时间线更新带上。 */
internal data class RoomCall(
    val widgets: List<RoomWidget> = emptyList(),
    val active: Boolean = false,
)

/**
 * 房间顶栏的 widget 入口：一个插件是一个按钮，图标按清单的 `entry`；装了几个时出菜单，名字用插件名。
 * 房间里有人在通话时按钮换成带“通话中”的样子，点了就是加入。
 */
@Composable
internal fun RoomCallEntry(call: RoomCall, onOpen: (RoomWidget) -> Unit) {
    val first = call.widgets.firstOrNull() ?: return
    var choosing by remember { mutableStateOf(false) }
    val onClick = { if (call.widgets.size == 1) onOpen(first) else choosing = true }
    Box {
        if (call.active) {
            ActiveCallButton(first.entry.icon(filled = true), onClick)
        } else {
            Box(Modifier.testTag("room-call")) {
                WontonIconButton(first.entry.icon(filled = false), first.entryLabel(), onClick)
            }
        }
        DropdownMenu(choosing, { choosing = false }, containerColor = WontonTheme.colors.surface.toColor()) {
            call.widgets.forEach { widget ->
                DropdownMenuItem(
                    text = { Text(widget.title) },
                    onClick = {
                        choosing = false
                        onOpen(widget)
                    },
                )
            }
        }
    }
}

@Composable
private fun ActiveCallButton(icon: ImageVector, onClick: () -> Unit) {
    val label = stringResource(R.string.room_call_active)
    val tint = WontonTheme.colors.success.toColor()
    Box(
        Modifier.heightIn(min = WontonTheme.size.target)
            .testTag("room-call-active")
            .clickable(role = Role.Button, onClick = onClick)
            .semantics(mergeDescendants = true) { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Row(
            Modifier.background(tint.copy(alpha = ACTIVE_FILL_ALPHA), CircleShape)
                .heightIn(min = WontonTheme.size.smallButton)
                .padding(horizontal = WontonTheme.spacing.s8),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // 装饰：外层已经带了名字。
            Icon(icon, contentDescription = null, Modifier.size(WontonTheme.size.iconSmall), tint = tint)
            Text(label, style = WontonTheme.typography.caption, color = tint, maxLines = 1)
        }
    }
}

private fun WidgetEntry.icon(filled: Boolean): ImageVector = when (this) {
    WidgetEntry.VIDEO_CALL -> if (filled) WontonIcon.VideoCallFill else WontonIcon.VideoCall
    WidgetEntry.VOICE_CALL -> if (filled) WontonIcon.VoiceCallFill else WontonIcon.VoiceCall
    WidgetEntry.OTHER -> if (filled) WontonIcon.AppsFill else WontonIcon.Apps
}

/** 读屏的名字：通话按种类说，别的 widget 用插件名。 */
@Composable
private fun RoomWidget.entryLabel(): String = when (entry) {
    WidgetEntry.VIDEO_CALL -> stringResource(R.string.room_call_video)
    WidgetEntry.VOICE_CALL -> stringResource(R.string.room_call_voice)
    WidgetEntry.OTHER -> title
}
