package app.wonton.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.core.view.ViewCompat
import app.wonton.protocol.Action
import app.wonton.protocol.TabDisplayChoice
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState
import app.wonton.ui.design.LocalWontonBarStatus
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Expand
import app.wonton.ui.icons.More
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.WontonIcon


@Composable
internal fun FullscreenTabBar(
    tab: TabRef, page: TabPage, tabs: TabsState, safe: PaddingValues,
    onClose: () -> Unit, onReload: () -> Unit, onDetails: (String) -> Unit,
) {
    val status = LocalWontonBarStatus.current
    val colors = WontonTheme.colors
    val shortStatus = status.title ?: if (page.state is TabState.Upgrading) tabText("正在升级…", "Updating…") else null
    val top = fullscreenCapsuleTop(safe, shortStatus != null || status.line != null)
    Column(Modifier.fillMaxWidth().padding(
        start = safe.calculateLeftPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8,
        end = safe.calculateRightPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8,
        top = top - (WontonTheme.size.target - WontonTheme.size.smallButton) / 2,
    )) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            if (shortStatus != null) {
                Surface(Modifier.weight(1f), shape = RoundedCornerShape(WontonTheme.radii.control),
                    color = colors.surface.toColor()) {
                    Text(shortStatus, Modifier.padding(WontonTheme.spacing.s8),
                        style = WontonTheme.typography.supporting, color = colors.textPrimary.toColor(),
                        maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            } else Spacer(Modifier.weight(1f))
            FullscreenCapsule(tab, page, tabs, onClose, onReload, onDetails)
        }
        status.line?.let { line ->
            Surface(color = colors.surface.toColor(), shape = RoundedCornerShape(WontonTheme.radii.control)) {
                Box(Modifier.padding(WontonTheme.spacing.s8)) { line() }
            }
        }
    }
}

@Composable
private fun fullscreenCapsuleTop(safe: PaddingValues, shortStatus: Boolean): Dp {
    val density = LocalDensity.current
    val view = LocalView.current
    val safeTop = safe.calculateTopPadding()
    val cutout = ViewCompat.getRootWindowInsets(view)?.displayCutout
    val right = view.width - with(density) {
        (safe.calculateRightPadding(LayoutDirection.Ltr) + WontonTheme.spacing.s8).roundToPx()
    }
    val left = right - with(density) { (WontonTheme.size.target * 2).roundToPx() }
    val fits = safeTop >= WontonTheme.size.target + WontonTheme.spacing.s8 && !shortStatus &&
        cutout != null && cutout.boundingRects.none { it.left < right && it.right > left }
    return if (fits) (safeTop - WontonTheme.size.smallButton) / 2 else safeTop + WontonTheme.spacing.s8
}

@Composable
private fun FullscreenCapsule(
    tab: TabRef, page: TabPage, tabs: TabsState,
    onClose: () -> Unit, onReload: () -> Unit, onDetails: (String) -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    Box(contentAlignment = Alignment.Center) {
        Surface(Modifier.size(WontonTheme.size.target * 2, WontonTheme.size.smallButton),
            shape = RoundedCornerShape(WontonTheme.radii.pill), color = WontonTheme.colors.surface.toColor(),
            border = BorderStroke(WontonTheme.size.hairline, WontonTheme.colors.separator.toColor())) {}
        Row(Modifier.height(WontonTheme.size.target)) {
            Box {
                WontonIconButton(WontonIcon.More, tabText("页面菜单", "Page menu"), { menu = true })
                DropdownMenu(menu, { menu = false }, containerColor = WontonTheme.colors.surface.toColor()) {
                    DropdownMenuItem(text = { Text(tabText("重新加载", "Reload")) },
                        onClick = { menu = false; onReload() })
                    DropdownMenuItem(text = { Text(tabText("总是全屏", "Always fullscreen")) },
                        trailingIcon = { WontonSwitch(page.opensFullscreen, { enabled ->
                            menu = false
                            tabs.send(Action.SetTabDisplay(tab, displayChoice(enabled)))
                        }) },
                        onClick = {
                            menu = false
                            tabs.send(Action.SetTabDisplay(tab, displayChoice(!page.opensFullscreen)))
                        })
                    if (tab is TabRef.Plugin) DropdownMenuItem(text = { Text(tabText("插件详情", "App details")) },
                        onClick = { menu = false; onDetails(tab.pluginId) })
                }
            }
            WontonIconButton(WontonIcon.Close, tabText("退出全屏", "Exit fullscreen"), onClose)
        }
    }
}

@Composable
internal fun TabBarMenu(item: TabItem, tabs: TabsState, onEdit: () -> Unit, onDismiss: () -> Unit) {
    WontonSheet(onDismiss, item.tabTitle()) { close ->
        Column(Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonGroup {
                item.page?.let { page ->
                    WontonRow(tabText("全屏打开", "Open fullscreen"), onClick = {
                        close { tabs.send(Action.EnterFullscreen(item.tab)) }
                    })
                    Row(Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
                        verticalAlignment = Alignment.CenterVertically) {
                        Text(tabText("总是全屏", "Always fullscreen"), Modifier.weight(1f),
                            style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
                        val label = tabText("总是全屏", "Always fullscreen")
                        Box(Modifier.semantics { contentDescription = label }) {
                            WontonSwitch(page.opensFullscreen, { enabled ->
                                tabs.send(Action.SetTabDisplay(item.tab, displayChoice(enabled)))
                            })
                        }
                    }
                }
                WontonRow(tabText("编辑底栏", "Edit tab bar"), onClick = { close(onEdit) })
            }
        }
    }
}

private fun displayChoice(enabled: Boolean): TabDisplayChoice =
    if (enabled) TabDisplayChoice.FULLSCREEN else TabDisplayChoice.STANDARD

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PluginTabBar(
    page: TabPage, canGoBack: Boolean, overlay: Boolean, onBack: () -> Unit,
    onClose: () -> Unit, takeover: PageTakeover?, onExpand: () -> Unit, onReload: () -> Unit,
) {
    val colors = WontonTheme.colors
    val status = LocalWontonBarStatus.current
    Column(Modifier.fillMaxWidth().background(colors.surface.toColor())) {
        TopAppBar(
            title = { Text(status.title ?: page.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = { if (canGoBack) WontonIconButton(WontonIcon.Back, tabText("返回", "Back"), onBack) },
            actions = {
                takeover?.let { TakeoverButton(it) }
                WontonIconButton(WontonIcon.Expand, tabText("全屏", "Fullscreen"), onExpand)
                WontonIconButton(WontonIcon.Refresh, tabText("重新加载", "Reload"), onReload)
                if (overlay) WontonIconButton(WontonIcon.Close, tabText("关闭", "Close"), onClose)
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = colors.surface.toColor()),
        )
        status.line?.let { Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset)) { it() } }
        if (page.state is TabState.Upgrading) Text(
            tabText("正在升级…", "Updating…"), Modifier.padding(horizontal = WontonTheme.spacing.pageInset),
            color = colors.textSecondary.toColor(), style = WontonTheme.typography.supporting,
        )
    }
}
