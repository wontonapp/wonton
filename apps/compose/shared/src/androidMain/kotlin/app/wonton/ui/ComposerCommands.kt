package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.RoomCommand
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * 输入“/”时弹在输入框上方的命令列表（ADR-0109）：按名字前缀过滤，点一条就把核心给的整句发出去。
 * 房间里有几个 Bot 时在每个名字旁写上是哪个 Bot 的；表被截断了在末尾说一声还有几条没列出来。
 */
@Composable
internal fun ComposerCommands(
    commands: List<RoomCommand>,
    omitted: Int,
    onPick: (RoomCommand) -> Unit,
) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.card)
    val severalBots = commands.any { it.botUserId != commands.first().botUserId }
    Column(
        Modifier.padding(horizontal = WontonTheme.spacing.s4, vertical = WontonTheme.spacing.s4)
            .fillMaxWidth()
            .background(colors.surface.toColor(), shape)
            .border(WontonTheme.size.hairline, colors.separator.toColor(), shape)
            // 最多显示几行高，再多就在列表里滚。
            .heightIn(max = WontonTheme.size.target * COMMAND_ROWS)
            .verticalScroll(rememberScrollState())
            .padding(vertical = WontonTheme.spacing.s4),
    ) {
        commands.forEachIndexed { index, command ->
            if (index > 0) Divider()
            CommandRow(command, severalBots) { onPick(command) }
        }
        if (omitted > 0) {
            Text(
                stringResource(R.string.composer_commands_more, omitted),
                Modifier.padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
            )
        }
    }
}

@Composable
private fun Divider() {
    Box(
        Modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.s12)
            .height(WontonTheme.size.hairline)
            .background(WontonTheme.colors.separator.toColor()),
    )
}

@Composable
private fun CommandRow(command: RoomCommand, showsBot: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().testTag("command-${command.name}")
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                Text(
                    "/${command.name}",
                    // 名字先占剩下的宽度再接提示；名字短就不拉伸，提示紧跟着。
                    Modifier.weight(1f, fill = false),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                // 参数提示跟在名字后面。
                command.hint?.let { hint ->
                    Text(
                        hint,
                        style = WontonTheme.typography.caption,
                        color = colors.textSecondary.toColor(),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Text(
                command.description,
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (showsBot) {
            Text(
                command.botName,
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
                maxLines = 1,
            )
        }
    }
}

/** 列表最多显示几行命令。 */
private const val COMMAND_ROWS = 5
