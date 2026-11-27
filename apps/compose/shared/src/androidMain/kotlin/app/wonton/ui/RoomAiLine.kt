package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.BotRef
import app.wonton.protocol.BotView
import app.wonton.ui.PluginsState
import app.wonton.ui.design.Icon
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.WontonIcon

/**
 * 群里能用的 AI：在场的 Bot 一行小胶囊，点了开 Bot 资料；确定一个都没有时说明 @ 不会有人回应。
 * 私聊不显示：Bot 本来就在顶栏（ADR-0044）。成员名单没取全（roomAi 是 null）时在场的算不准，
 * 不下“群里没有”的结论，什么都不显示。
 */
@Composable
internal fun RoomAiLine(
    roomAi: List<BotView>?,
    plugins: PluginsState,
    onOpenBotProfile: (BotRef) -> Unit,
) {
    roomAi ?: return
    if (roomAi.isEmpty()) {
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
                stringResource(R.string.room_no_ai),
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
            )
        }
        return
    }
    Row(
        Modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        roomAi.forEach { bot ->
            Row(
                Modifier
                    .heightIn(min = WontonTheme.size.target)
                    .clip(RoundedCornerShape(WontonTheme.radii.pill))
                    .clickable(role = Role.Button) { onOpenBotProfile(bot.bot) }
                    .padding(horizontal = WontonTheme.spacing.s8, vertical = WontonTheme.spacing.s4),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            ) {
                BotAvatar(bot, plugins, WontonTheme.size.icon)
                Text(
                    bot.name,
                    style = WontonTheme.typography.caption,
                    color = WontonTheme.colors.textPrimary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}
