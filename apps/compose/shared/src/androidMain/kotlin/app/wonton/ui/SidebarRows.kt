package app.wonton.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.SidebarApp
import app.wonton.protocol.SidebarOpen
import app.wonton.protocol.TabRef
import app.wonton.ui.icons.PinFill
import app.wonton.ui.icons.WontonIcon
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private const val BADGE_MAX = 99

internal val UPDATE_DOT = 8.dp
private val BADGE_MIN = 22.dp
private val BADGE_PAD = 7.dp

/** 侧边栏一行的高：44（iOS 原型也是）。 */
private val ROW_HEIGHT = 44.dp

/** 一行能打开的应用：插件图标加名字，可更新的带红点，固定了的带图钉；长按固定或拿掉。数据是核心算好的。 */
@Composable
internal fun SidebarAppRow(
    app: SidebarApp,
    plugins: PluginsState,
    tabs: TabsState,
    current: TabRef,
    interactive: Boolean,
    onGo: (SidebarDestination) -> Unit,
) {
    // 长按菜单用的页面清单还在已安装列表里（核心只给点开的那一个）。
    val plugin = plugins.viewModel.installed?.plugins?.firstOrNull { it.id == app.pluginId }
    val choice = (app.open as? SidebarOpen.Page)?.tab
    SidebarRow(
        title = app.title,
        selected = choice != null && choice == current,
        interactive = interactive,
        badge = if (app.update) 1 else 0,
        dotOnly = true,
        pinned = choice != null && tabs.viewModel.items.any { it.tab == choice },
        onClick = {
            when (val open = app.open) {
                is SidebarOpen.BotChat -> onGo(SidebarDestination.Plugin(PluginEntry.Chat(open.botUserId)))
                is SidebarOpen.Page -> onGo(SidebarDestination.Plugin(PluginEntry.Web(open.tab)))
            }
        },
        icon = { PluginIconImage(app.icon, ROW_ICON, plugins) },
        menu = plugin?.tabChoices?.takeIf { it.isNotEmpty() }?.let {
            { onDismiss -> PinMenuItems(plugin, tabs, onDismiss) }
        },
    )
}

/** 长按菜单：每个页面一项，没固定的固定到底栏（满了走现有的替换面板），拿得掉的从底栏拿掉。 */
@Composable
private fun PinMenuItems(plugin: InstalledPlugin, tabs: TabsState, onDismiss: () -> Unit) {
    plugin.tabChoices.forEach { choice ->
        val item = tabs.viewModel.items.firstOrNull { it.tab == choice.tab }
        when {
            item == null -> DropdownMenuItem(
                text = { Text(stringResource(R.string.pin_to_bar)) },
                onClick = {
                    onDismiss()
                    tabs.pin(choice.tab, choice.title)
                },
            )

            item.removable -> DropdownMenuItem(
                text = { Text(stringResource(R.string.unpin_from_bar)) },
                onClick = {
                    onDismiss()
                    tabs.send(Action.UnpinTab(choice.tab))
                },
            )
        }
    }
}

/** 侧边栏的一行：图标、文字，选中一块圆角底色；行尾是图钉、红点或数字角标。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun SidebarRow(
    title: String,
    selected: Boolean,
    interactive: Boolean,
    onClick: () -> Unit,
    icon: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    badge: Int = 0,
    dotOnly: Boolean = false,
    pinned: Boolean = false,
    menu: (@Composable (onDismiss: () -> Unit) -> Unit)? = null,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    var expanded by remember { mutableStateOf(false) }
    Box(modifier) {
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = ROW_HEIGHT)
                .clip(RoundedCornerShape(WontonTheme.radii.control))
                .background(if (selected) colors.surface.toColor() else Color.Transparent)
                .combinedClickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                    enabled = interactive,
                    role = Role.Button,
                    onLongClick = ({ expanded = true }).takeIf { menu != null },
                    onClick = onClick,
                )
                .padding(horizontal = spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.s12),
        ) {
            icon()
            Text(
                title,
                Modifier.weight(1f),
                style = WontonTheme.typography.body.copy(
                    fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
                ),
                color = colors.textPrimary.toColor(),
                maxLines = 1,
            )
            if (pinned) {
                Icon(
                    WontonIcon.PinFill,
                    contentDescription = stringResource(R.string.sidebar_pinned),
                    Modifier.size(WontonTheme.size.iconTiny),
                    tint = colors.textSecondary.toColor(),
                )
            }
            if (badge > 0) {
                if (dotOnly) {
                    val updatableLabel = stringResource(R.string.sidebar_updatable)
                    Box(
                        Modifier.size(UPDATE_DOT).background(colors.danger.toColor(), CircleShape)
                            .semantics { contentDescription = updatableLabel },
                    )
                } else {
                    Box(
                        Modifier.background(colors.danger.toColor(), RoundedCornerShape(WontonTheme.radii.pill))
                            .widthIn(min = BADGE_MIN)
                            .heightIn(min = BADGE_MIN)
                            .wrapContentSize(Alignment.Center)
                            .padding(horizontal = BADGE_PAD),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            if (badge > BADGE_MAX) "$BADGE_MAX+" else badge.toString(),
                            style = WontonTheme.typography.caption.copy(fontWeight = FontWeight.SemiBold),
                            color = Color.White,
                        )
                    }
                }
            }
        }
        if (menu != null && expanded) {
            DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                menu { expanded = false }
            }
        }
    }
}
