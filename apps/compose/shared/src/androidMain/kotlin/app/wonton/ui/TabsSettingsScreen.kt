package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.TabCandidate
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabsViewModel
import app.wonton.ui.design.TabBarIcon
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

private const val DRAG_HANDLE_LINES = 3
private const val DRAG_HANDLE_STROKE_SCALE = 1f / 12f
private const val DRAG_HANDLE_INSET = 1f / 6f

@Composable
fun TabsSettingsScreen(viewModel: TabsViewModel, onBack: () -> Unit) {
    var adding by remember { mutableStateOf(false) }
    if (adding) {
        TabCandidatesScreen(viewModel) { adding = false }
        return
    }
    BackHandler(onBack = onBack)
    val list = rememberLazyListState()
    WontonScreen(bar = { TabSettingsBar(tabText("底栏", "Tab bar"), onBack) }) {
        LazyColumn(
            state = list,
            modifier = Modifier.fillMaxSize().padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            item(key = "labels") { WontonGroup { TabLabelsRow(viewModel.labels) } }
            item(key = "current") { CurrentTabsGroup(viewModel.items, list) }
            if (viewModel.candidates.isNotEmpty()) {
                item(key = "add") {
                    WontonGroup {
                        WontonRow(tabText("添加更多", "Add more"), showsForward = true, onClick = { adding = true })
                    }
                }
            }
            item(key = "problem") { TabsConfigurationProblem(viewModel) }
        }
    }
}

@Composable
private fun TabLabelsRow(visible: Boolean) {
    val tabs = LocalTabs.current
    val title = tabText("显示文字", "Show labels")
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(title, Modifier.weight(1f), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor())
        Box(Modifier.semantics { contentDescription = title }) {
            WontonSwitch(visible, { tabs.send(Action.SetTabLabels(it)) })
        }
    }
}

/** Only the rows inside the group participate in drag destinations, not the lazy list's other items. */
@Composable
private fun CurrentTabsGroup(items: List<TabItem>, list: LazyListState) {
    val tabs = LocalTabs.current
    val scope = rememberCoroutineScope()
    val edgeSize = WontonTheme.size.target
    val heights = remember { mutableMapOf<TabRef, Int>() }
    var dragging by remember(items) { mutableStateOf<TabRef?>(null) }
    var displacement by remember { mutableStateOf(0f) }
    var pointerY by remember { mutableStateOf(0f) }
    var destination by remember { mutableStateOf(0) }
    fun locateDestination() {
        var bottom = 0
        destination = items.indexOfFirst { item ->
            bottom += heights[item.tab] ?: 0
            pointerY < bottom
        }.let { if (it < 0) items.lastIndex else it }
    }
    WontonGroup {
        items.forEachIndexed { index, item ->
            val modifier = Modifier.onSizeChanged { heights[item.tab] = it.height }
                .zIndex(if (dragging == item.tab) 1f else 0f)
                .graphicsLayer { translationY = if (dragging == item.tab) displacement else 0f }
                .pointerInput(items) {
                    detectDragGesturesAfterLongPress(
                        onDragStart = { offset ->
                            dragging = item.tab
                            displacement = 0f
                            pointerY = (0 until index).sumOf { heights[items[it].tab] ?: 0 } + offset.y
                            destination = index
                        },
                        onDragCancel = { dragging = null },
                        onDragEnd = {
                            if (destination != index) tabs.send(Action.MoveTab(item.tab, destination.toUInt()))
                            dragging = null
                        },
                        onDrag = { change, amount ->
                            change.consume()
                            displacement += amount.y
                            pointerY += amount.y
                            locateDestination()
                            val layout = list.layoutInfo
                            val group = layout.visibleItemsInfo.firstOrNull { it.key == "current" }
                            val viewportY = (group?.offset ?: 0) + pointerY
                            val edge = edgeSize.toPx()
                            val scroll = when {
                                viewportY < layout.viewportStartOffset + edge -> -edge / 3
                                viewportY > layout.viewportEndOffset - edge -> edge / 3
                                else -> 0f
                            }
                            if (scroll != 0f) scope.launch {
                                val consumed = list.scrollBy(scroll)
                                if (dragging == item.tab) {
                                    displacement += consumed
                                    pointerY += consumed
                                    locateDestination()
                                }
                            }
                        },
                    )
                }
            key(item.tab) { TabSettingsRow(item, index, items.size, modifier) }
        }
    }
}

@Composable
private fun TabSettingsRow(item: TabItem, index: Int, count: Int, modifier: Modifier) {
    val tabs = LocalTabs.current
    val moveUp = tabText("上移", "Move up")
    val moveDown = tabText("下移", "Move down")
    val colors = WontonTheme.colors
    Row(
        modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .semantics(mergeDescendants = true) {
                customActions = buildList {
                    if (index > 0) add(CustomAccessibilityAction(moveUp) {
                        tabs.send(Action.MoveTab(item.tab, (index - 1).toUInt())); true
                    })
                    if (index < count - 1) add(CustomAccessibilityAction(moveDown) {
                        tabs.send(Action.MoveTab(item.tab, (index + 1).toUInt())); true
                    })
                }
            }.padding(start = WontonTheme.spacing.s16, end = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        TabDragHandle()
        TabBarIcon(item, false, colors.textPrimary.toColor(), tabs.plugins)
        Text(item.tabTitle(), Modifier.weight(1f).padding(vertical = WontonTheme.spacing.s12),
            style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        if (item.removable) {
            WontonIconButton(WontonIcon.Trash, tabText("从底栏移除", "Remove from tab bar"),
                { tabs.send(Action.UnpinTab(item.tab)) }, tint = colors.danger.toColor())
        }
    }
}

@Composable
private fun TabDragHandle() {
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
private fun TabCandidatesScreen(viewModel: TabsViewModel, onBack: () -> Unit) {
    BackHandler(onBack = onBack)
    WontonScreen(bar = { TabSettingsBar(tabText("添加到底栏", "Add to tab bar"), onBack) }) {
        LazyColumn(Modifier.fillMaxSize().padding(horizontal = WontonTheme.spacing.pageInset)) {
            items(viewModel.candidates, key = { it.tab.toString() }) { candidate ->
                WontonGroup { TabCandidateRow(candidate) }
            }
            item { TabsConfigurationProblem(viewModel) }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TabSettingsBar(title: String, onBack: () -> Unit) {
    TopAppBar(
        title = { Text(title) },
        navigationIcon = { WontonIconButton(WontonIcon.Back, tabText("返回", "Back"), onBack) },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = WontonTheme.colors.surface.toColor()),
    )
}

@Composable
private fun TabCandidateRow(candidate: TabCandidate) {
    val tabs = LocalTabs.current
    val plugin = candidate.plugin
    val title = if (plugin == null) tabText("设置", "Settings") else {
        plugin.pageTitle?.let { "${plugin.title} · $it" } ?: plugin.title
    }
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button) { tabs.pin(candidate.tab, title) }
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        if (plugin != null) PluginIconImage(plugin.icon, WontonTheme.size.icon, tabs.plugins)
        else Icon(WontonIcon.Settings, null, Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor())
        Text(title, Modifier.weight(1f), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor())
        Icon(WontonIcon.Add, null, Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor())
    }
}

@Composable
internal fun TabsConfigurationProblem(viewModel: TabsViewModel) {
    viewModel.problem?.let {
        ProblemNotice(Problem(
            tabText("读不了底栏设置", "Could not read tab settings"),
            tabText("已显示默认底栏。修改底栏后会重新保存设置。", "Showing default tabs. Changes will save the settings again."),
            it.message,
        ))
    }
}
