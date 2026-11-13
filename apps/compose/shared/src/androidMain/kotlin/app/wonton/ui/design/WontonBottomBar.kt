package app.wonton.ui.design

import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.TabIcon
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabsViewModel
import app.wonton.ui.PluginIconImage
import app.wonton.ui.PluginsState
import app.wonton.ui.R
import app.wonton.ui.icons.Apps
import app.wonton.ui.icons.AppsFill
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.ChatFill
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.SettingsFill
import app.wonton.ui.icons.WontonIcon
import app.wonton.ui.rememberPluginImage
import app.wonton.ui.tabTitle

private const val DIMMED_ALPHA = 0.4f

@Composable
fun WontonBottomBar(
    tabs: TabsViewModel, plugins: PluginsState, onSelect: (TabRef) -> Unit, onMenu: (TabRef) -> Unit,
) {
    val colors = WontonTheme.colors
    val menuLabel = stringResource(R.string.design_tab_menu)
    NavigationBar(
        modifier = Modifier.navigationBarsPadding()
            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8)
            .clip(RoundedCornerShape(percent = 50)),
        containerColor = colors.surface.toColor().copy(alpha = 0.85f),
        windowInsets = WindowInsets(0, 0, 0, 0),
        tonalElevation = 0.dp,
    ) {
        tabs.items.forEach { item ->
            val selected = item.tab == tabs.selected
            val title = item.tabTitle()
            val ink = (if (selected) colors.accentInkOnFill else colors.textSecondary).toColor()
            NavigationBarItem(
                selected = selected,
                onClick = { onSelect(item.tab) },
                modifier = Modifier.pointerInput(onMenu, item.tab) {
                    awaitEachGesture {
                        awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                        val up = withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                            waitForUpOrCancellation(pass = PointerEventPass.Initial)
                        }
                        if (up == null) {
                            onMenu(item.tab)
                            do {
                                val event = awaitPointerEvent(PointerEventPass.Initial)
                                event.changes.forEach { it.consume() }
                            } while (event.changes.any { it.pressed })
                        }
                    }
                }.semantics {
                    contentDescription = title
                    customActions = listOf(CustomAccessibilityAction(menuLabel) { onMenu(item.tab); true })
                },
                icon = {
                    BadgedBox(badge = {
                        if (item.badge) Badge(containerColor = colors.danger.toColor())
                    }) {
                        Box(Modifier.alpha(if (item.page?.dimmed == true) DIMMED_ALPHA else 1f)) {
                            TabBarIcon(item, selected, ink, plugins)
                        }
                    }
                },
                label = if (tabs.labels) {
                    { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                } else null,
                alwaysShowLabel = tabs.labels,
                colors = NavigationBarItemDefaults.colors(
                    selectedIconColor = colors.accentInkOnFill.toColor(),
                    selectedTextColor = colors.textPrimary.toColor(),
                    indicatorColor = colors.accentFill.toColor(),
                    unselectedIconColor = colors.textSecondary.toColor(),
                    unselectedTextColor = colors.textSecondary.toColor(),
                ),
            )
        }
    }
}

@Composable
internal fun TabBarIcon(item: TabItem, selected: Boolean, tint: Color, plugins: PluginsState) {
    val size = WontonTheme.size.icon
    val builtIn = when (item.tab) {
        TabRef.Chat -> if (selected) WontonIcon.ChatFill else WontonIcon.Chat
        TabRef.Apps -> if (selected) WontonIcon.AppsFill else WontonIcon.Apps
        TabRef.Settings -> if (selected) WontonIcon.SettingsFill else WontonIcon.Settings
        is TabRef.Plugin, is TabRef.PluginPath, is TabRef.Bot, is TabRef.Url, is TabRef.SharedApp,
        is TabRef.UiResource, is TabRef.Widget -> null
    }
    if (builtIn != null) {
        // 装饰：底栏每一格都有标题。
        Icon(builtIn, null, Modifier.size(size), tint)
        return
    }
    when (val icon = item.page?.icon) {
        is TabIcon.Plugin -> PluginIconImage(icon.icon, size, plugins)
        is TabIcon.Monochrome -> {
            val image = rememberPluginImage(plugins, if (selected) icon.selected ?: icon.icon else icon.icon)
            // 装饰：底栏每一格都有标题。
            if (image != null) Image(image, null, Modifier.size(size), colorFilter = ColorFilter.tint(tint))
        }
        // 装饰：底栏每一格都有标题。
        null -> Icon(WontonIcon.Apps, null, Modifier.size(size), tint)
    }
}
