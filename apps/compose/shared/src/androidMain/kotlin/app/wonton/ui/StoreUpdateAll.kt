package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.UpdatablePlugin
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Download
import app.wonton.ui.icons.WontonIcon

/** 商店右上角的更新图标（向下的箭头），有可更新时才出现，带红色数字角标。 */
@Composable
internal fun UpdateAllButton(count: Int, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val label = stringResource(R.string.store_update_all_a11y, count)
    Box(
        Modifier.size(WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            WontonIcon.Download,
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = colors.textPrimary.toColor(),
        )
        Text(
            if (count > UPDATE_ALL_BADGE_MAX) "$UPDATE_ALL_BADGE_MAX+" else count.toString(),
            Modifier.align(Alignment.TopEnd)
                .offset(x = UPDATE_BADGE_OFFSET_X, y = UPDATE_BADGE_OFFSET_Y)
                .background(colors.danger.toColor(), CircleShape)
                .padding(horizontal = 5.dp),
            style = WontonTheme.typography.caption.copy(fontWeight = FontWeight.Bold),
            color = Color.White,
        )
    }
}

/** 全部更新前唯一的一次确认：列出要更新的应用和新版本号，防误触。 */
@Composable
internal fun UpdateAllSheet(
    plugins: PluginsState,
    updating: List<UpdatablePlugin>,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    WontonSheet(onDismiss, stringResource(R.string.store_update_all_title, updating.size)) { close ->
        Column(
            Modifier.fillMaxWidth().padding(horizontal = spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(spacing.s16),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(spacing.s12)) {
                updating.forEach { app ->
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(spacing.s12),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        PluginIconImage(app.icon, WontonTheme.size.taskIcon, plugins)
                        Text(
                            app.title,
                            Modifier.weight(1f),
                            style = WontonTheme.typography.body,
                            color = colors.textPrimary.toColor(),
                        )
                        Text(
                            app.version,
                            style = WontonTheme.typography.supporting,
                            color = colors.textSecondary.toColor(),
                        )
                    }
                }
            }
            WontonButton(
                stringResource(R.string.store_update_all),
                { close { onConfirm() } },
                Modifier.fillMaxWidth(),
            )
        }
    }
}

private const val UPDATE_ALL_BADGE_MAX = 99

private val UPDATE_BADGE_OFFSET_X = (-4).dp
private val UPDATE_BADGE_OFFSET_Y = 6.dp
