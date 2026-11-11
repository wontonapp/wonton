package app.wonton.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.CandidatePlugin
import app.wonton.protocol.PanelScope
import app.wonton.protocol.TabCandidate
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Globe
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.ImeAction

private const val DRAG_HANDLE_LINES = 3
private const val DRAG_HANDLE_STROKE_SCALE = 1f / 12f
private const val DRAG_HANDLE_INSET = 1f / 6f

/**
 * 编辑面板（ADR-0052）：上面是面板里的页面，能拖动排序、拿掉；下面是还能放进来的页面和一行填写网址。
 * 主面板和房间面板用同一个表单，只改 [app.wonton.protocol.PanelScope]。
 */
@Composable
internal fun ChatPanelEditor(panel: ChatPanelState, tabs: TabsState, onDismiss: () -> Unit) {
    var filling by remember { mutableStateOf(false) }
    WontonSheet(onDismiss, stringResource(R.string.panel_edit)) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState())) {
            if (panel.pages.isNotEmpty()) {
                Section(stringResource(R.string.panel_in_panel))
                PanelEntries(panel, tabs)
            }
            if (panel.pages.size < PANEL_MAX_PAGES) {
                Section(stringResource(R.string.panel_can_add))
                WontonGroup {
                    WontonRow(stringResource(R.string.panel_url), showsForward = true, onClick = { filling = true })
                }
                Column(Modifier.padding(top = WontonTheme.spacing.s8)) {
                    val candidates = candidates(panel, tabs).take(PANEL_MAX_PAGES - panel.pages.size)
                    candidates.forEach { candidate ->
                        key(candidate.tab) { WontonGroup { CandidateRow(candidate, tabs, panel) } }
                    }
                }
            }
        }
    }
    if (filling) UrlDialog(tabs, panel) { filling = false }
}

private const val PANEL_MAX_PAGES = 5

@Composable
private fun Section(title: String) {
    Text(
        title,
        Modifier.padding(start = WontonTheme.spacing.s16, top = WontonTheme.spacing.s16,
            bottom = WontonTheme.spacing.s8, end = WontonTheme.spacing.s16),
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

/**
 * 还能放进这个面板的页。主面板直接取核心的候选；房间面板的候选核心只给了主面板那份
 * （不含已经在主面板里的页），这里按核心同样的规则（已装插件、有网页、声明了的页）补全，再去掉房间面板里已有的。
 */
private fun candidates(panel: ChatPanelState, tabs: TabsState): List<TabCandidate> {
    val offered = if (panel.scope is PanelScope.Room) {
        val known = tabs.viewModel.panelCandidates.map { it.tab }.toSet()
        tabs.viewModel.panelCandidates + tabs.plugins.viewModel.installed?.plugins.orEmpty()
            .filter { it.webUrl != null }
            .flatMap { plugin ->
                val pages = if (plugin.tabs.isEmpty()) listOf(null) else plugin.tabs
                pages.mapNotNull { page ->
                    val tab = TabRef.Plugin(plugin.id, page?.id)
                    if (tab in known) {
                        null
                    } else {
                        TabCandidate(tab, CandidatePlugin(
                            plugin.title, page?.takeIf { plugin.tabs.size > 1 }?.title, plugin.icon,
                        ))
                    }
                }
            }
    } else {
        tabs.viewModel.panelCandidates
    }
    return offered.filter { candidate -> panel.pages.none { it.tab == candidate.tab } }
}

@Composable
private fun PanelEntries(panel: ChatPanelState, tabs: TabsState) {
    val colors = WontonTheme.colors
    val heights = remember { mutableMapOf<TabRef, Int>() }
    var dragging by remember(panel.pages) { mutableStateOf<TabRef?>(null) }
    var displacement by remember { mutableStateOf(0f) }
    var destination by remember { mutableStateOf(0) }
    fun locate(pointerY: Float) {
        var bottom = 0
        destination = panel.pages.indexOfFirst { item ->
            bottom += heights[item.tab] ?: 0
            pointerY < bottom
        }.let { if (it < 0) panel.pages.lastIndex else it }
    }
    WontonGroup {
        panel.pages.forEachIndexed { index, item ->
            val moveUp = stringResource(R.string.tab_move_up)
            val moveDown = stringResource(R.string.tab_move_down)
            Row(
                Modifier.fillMaxWidth()
                    .onSizeChanged { heights[item.tab] = it.height }
                    .zIndex(if (dragging == item.tab) 1f else 0f)
                    .graphicsLayer { translationY = if (dragging == item.tab) displacement else 0f }
                    .pointerInput(panel.pages) {
                        detectDragGesturesAfterLongPress(
                            onDragStart = { offset ->
                                dragging = item.tab
                                displacement = 0f
                                locate((0 until index).sumOf { heights[panel.pages[it].tab] ?: 0 } + offset.y)
                                destination = index
                            },
                            onDragCancel = { dragging = null },
                            onDragEnd = {
                                if (destination != index) {
                                    tabs.send(Action.MovePanelPage(panel.scope, item.tab, destination.toUInt()))
                                }
                                dragging = null
                            },
                            onDrag = { change, amount ->
                                change.consume()
                                displacement += amount.y
                                locate(
                                    panel.pages.take(index).sumOf { heights[it.tab] ?: 0 } + displacement,
                                )
                            },
                        )
                    }
                    .semantics(mergeDescendants = true) {
                        customActions = buildList {
                            if (index > 0) add(CustomAccessibilityAction(moveUp) {
                                tabs.send(Action.MovePanelPage(panel.scope, item.tab, (index - 1).toUInt())); true
                            })
                            if (index < panel.pages.lastIndex) add(CustomAccessibilityAction(moveDown) {
                                tabs.send(Action.MovePanelPage(panel.scope, item.tab, (index + 1).toUInt())); true
                            })
                        }
                    }
                    .padding(start = WontonTheme.spacing.s16, end = WontonTheme.spacing.s8),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                DragHandle()
                EntryIcon(item.tab, tabs)
                Text(
                    item.page.title,
                    Modifier.weight(1f).padding(vertical = WontonTheme.spacing.s12),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
                WontonIconButton(
                    WontonIcon.Trash, stringResource(R.string.panel_remove),
                    { tabs.send(Action.RemovePanelPage(panel.scope, item.tab)) }, tint = colors.danger.toColor(),
                )
            }
        }
    }
}

/** 行首的来源图标：公网网页是地球，插件页是插件自己的图标。 */
@Composable
private fun EntryIcon(tab: TabRef, tabs: TabsState) {
    when (tab) {
        is TabRef.Url -> Icon(
            WontonIcon.Globe, null, Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
        is TabRef.Plugin, is TabRef.PluginPath -> {
            val id = (tab as? TabRef.Plugin)?.pluginId ?: (tab as? TabRef.PluginPath)?.pluginId
            val icon = tabs.plugins.viewModel.installed?.plugins?.firstOrNull { it.id == id }?.icon
            PluginIconImage(icon, WontonTheme.size.icon, tabs.plugins)
        }
        else -> Unit
    }
}

@Composable
private fun DragHandle() {
    val ink = WontonTheme.colors.textSecondary.toColor()
    Canvas(Modifier.size(WontonTheme.size.iconSmall)) {
        val stroke = size.height * DRAG_HANDLE_STROKE_SCALE
        val inset = size.width * DRAG_HANDLE_INSET
        for (line in 1..DRAG_HANDLE_LINES) {
            val y = size.height * line / (DRAG_HANDLE_LINES + 1)
            drawLine(ink, Offset(inset, y), Offset(size.width - inset, y), stroke)
        }
    }
}

@Composable
private fun CandidateRow(candidate: TabCandidate, tabs: TabsState, panel: ChatPanelState) {
    val plugin = candidate.plugin ?: return
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button) { tabs.send(Action.AddPanelPage(panel.scope, candidate.tab)) }
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        PluginIconImage(plugin.icon, WontonTheme.size.icon, tabs.plugins)
        Text(
            plugin.pageTitle?.let { "${plugin.title} · $it" } ?: plugin.title,
            Modifier.weight(1f),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        Icon(WontonIcon.Add, null, Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor())
    }
}

/** 填写网址：只收 https，核心再校验一次。填完直接加，失败的原因由核心弹窗说。 */
@Composable
private fun UrlDialog(tabs: TabsState, panel: ChatPanelState, onDismiss: () -> Unit) {
    var url by remember { mutableStateOf("") }
    WontonDialog(
        title = stringResource(R.string.panel_url),
        onDismiss = onDismiss,
        detail = {
            WontonTextField(
                url,
                { url = it },
                Modifier.fillMaxWidth(),
                placeholder = "https://example.com",
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
            )
        },
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(
                stringResource(R.string.panel_add),
                WontonButtonKind.PRIMARY,
                onClick = {
                    val typed = url.trim()
                    val full = if ("://" in typed) typed else "https://$typed"
                    tabs.send(Action.AddPanelPage(panel.scope, TabRef.Url(full)))
                    onDismiss()
                },
            ),
        ),
    )
}
