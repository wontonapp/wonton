package app.wonton.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import app.wonton.protocol.SidebarApp
import app.wonton.protocol.SidebarOpen
import app.wonton.protocol.TabRef
import app.wonton.ui.design.Icon
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.PauseFill
import app.wonton.ui.icons.WontonIcon

private const val BADGE_MAX = 99

internal val UPDATE_DOT = 8.dp
private val BADGE_MIN = 22.dp
private val BADGE_PAD = 7.dp

/** 侧边栏一行的高：44（iOS 原型也是）。 */
private val ROW_HEIGHT = 44.dp

/** "我的应用"的一行：插件图标加名字，行尾红点核心算好；长按出这一页的菜单（[MyAppMenuItems]）。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun SidebarAppRow(
    app: SidebarApp,
    plugins: PluginsState,
    pages: PagesState,
    current: TabRef,
    interactive: Boolean,
    onGo: (SidebarDestination) -> Unit,
) {
    // 长按菜单用的页面清单还在已安装列表里（核心只给点开的那一个）。
    val plugin = plugins.viewModel.installed?.plugins?.firstOrNull { it.id == app.pluginId }
    val choice = (app.open as? SidebarOpen.Page)?.tab
    // 这两类行出门就交给系统浏览器：先在行上讲明，点开时不会以为是内嵌页面（ADR-0033、ADR-0080）。
    // 条件照核心的 `browser::opens_in_browser` 抄（这边只有 Android，省掉引擎判断），改规则时两处一起改；
    // `Page` 已限定为插件的网页页，BotChat 行点开是聊天、`ExternalBrowser` 本来就直开浏览器。
    val inBrowser = app.open is SidebarOpen.ExternalBrowser ||
        (app.open is SidebarOpen.Page && plugin?.crossOriginIsolated == true && plugin.webUrl != null)
    SidebarRow(
        title = app.title,
        selected = choice != null && choice == current,
        interactive = interactive,
        // 交给系统浏览器的提示比用途要紧，两样都有时只放得下一行，留提示。
        subtitle = if (inBrowser) stringResource(R.string.sidebar_opens_in_browser) else app.subtitle,
        badgeMark = SidebarAppBadge(app.badge.update, app.badge.problem, app.badge.stopped),
        onClick = {
            when (val open = app.open) {
                is SidebarOpen.BotChat -> onGo(SidebarDestination.Plugin(PluginEntry.Chat(open.botUserId)))
                is SidebarOpen.Page -> onGo(SidebarDestination.Plugin(PluginEntry.Web(open.tab)))
                is SidebarOpen.ExternalBrowser -> onGo(SidebarDestination.Plugin(PluginEntry.Browser(open.url)))
            }
        },
        icon = { PluginIconImage(app.icon, ROW_ICON, plugins) },
        menu = MyAppMenu(app, plugin, inBrowser, pages, onGo).takeIf { it.hasItems }?.let { menu ->
            { onDismiss -> MyAppMenuItems(menu, onDismiss) }
        },
    )
}

/** 侧边栏的一行：图标、文字，选中一块圆角底色；行尾是红点或数字角标。 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun SidebarRow(
    title: String,
    selected: Boolean,
    interactive: Boolean,
    onClick: () -> Unit,
    icon: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    badge: Int = 0,
    dotOnly: Boolean = false,
    badgeMark: SidebarAppBadge? = null,
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
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    title,
                    style = WontonTheme.typography.body.copy(
                        fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
                    ),
                    color = colors.textPrimary.toColor(),
                    maxLines = 1,
                )
                subtitle?.let {
                    Text(
                        it,
                        style = WontonTheme.typography.supporting,
                        color = colors.textSecondary.toColor(),
                        maxLines = 1,
                    )
                }
            }
            badgeMark?.let { mark ->
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (mark.stopped) {
                        Icon(
                            WontonIcon.PauseFill,
                            contentDescription = stringResource(R.string.launcher_badge_stopped),
                            modifier = Modifier.size(WontonTheme.size.iconSmall),
                            tint = colors.textSecondary.toColor(),
                        )
                    }
                    if (mark.update) {
                        val updatableLabel = stringResource(R.string.sidebar_updatable)
                        Box(
                            Modifier.size(UPDATE_DOT).background(colors.danger.toColor(), CircleShape)
                                .semantics { contentDescription = updatableLabel },
                        )
                    }
                    if (mark.problem) {
                        val problemLabel = stringResource(R.string.sidebar_problem)
                        Box(
                            Modifier.size(UPDATE_DOT).background(colors.warning.toColor(), CircleShape)
                                .semantics { contentDescription = problemLabel },
                        )
                    }
                }
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

/** 核心给的状态标记独立呈现，不让停止状态遮住更新或问题。 */
internal data class SidebarAppBadge(val update: Boolean, val problem: Boolean, val stopped: Boolean)
