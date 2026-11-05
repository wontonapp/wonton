package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.TabItem
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabsViewModel
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.launch

/** 顺序只由核心更新；拖动中的位移不是一份本地底栏配置。 */
@Composable
fun TabsSettingsScreen(viewModel: TabsViewModel, onBack: () -> Unit) {
    val tabs = LocalTabs.current
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    var dragging by remember { mutableStateOf<TabRef?>(null) }
    var displacement by remember { mutableStateOf(0f) }
    var pointerY by remember { mutableStateOf(0f) }
    var destination by remember { mutableStateOf<Int?>(null) }
    val choices = listOf(TabChoice(TabRef.Settings, tabText("设置", "Settings"))) +
        tabs.plugins.viewModel.installed?.plugins.orEmpty().flatMap { it.tabChoices }
    BackHandler(onBack = onBack)
    WontonScreen(bar = { WontonPushedBar(tabText("底栏", "Tab bar"), onBack) }) {
        Column(Modifier.fillMaxSize().padding(WontonTheme.spacing.pageInset)) {
            TabsConfigurationProblem(viewModel)
            LazyColumn(
                state = list,
                modifier = Modifier.fillMaxSize(),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                itemsIndexed(viewModel.items, key = { _, item -> item.tab.toString() }) { index, item ->
                    val dragModifier = Modifier.zIndex(if (dragging == item.tab) 1f else 0f)
                        .graphicsLayer { translationY = if (dragging == item.tab) displacement else 0f }
                        .pointerInput(viewModel.items) {
                            detectDragGesturesAfterLongPress(
                                onDragStart = { offset ->
                                    dragging = item.tab
                                    displacement = 0f
                                    pointerY = (list.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }
                                        ?.offset ?: 0) + offset.y
                                    destination = index
                                },
                                onDragCancel = { dragging = null },
                                onDragEnd = {
                                    destination?.takeIf { it != index }?.let {
                                        tabs.send(Action.MoveTab(item.tab, it.toUInt()))
                                    }
                                    dragging = null
                                },
                                onDrag = { change, amount ->
                                    change.consume()
                                    displacement += amount.y
                                    pointerY += amount.y
                                    val layout = list.layoutInfo
                                    layout.visibleItemsInfo.firstOrNull {
                                        pointerY >= it.offset && pointerY < it.offset + it.size
                                    }?.index?.takeIf { it < viewModel.items.size }?.let { destination = it }
                                    val edge = 48.dp.toPx()
                                    val scroll = when {
                                        pointerY < layout.viewportStartOffset + edge -> -edge / 3
                                        pointerY > layout.viewportEndOffset - edge -> edge / 3
                                        else -> 0f
                                    }
                                    if (scroll != 0f) scope.launch { list.scrollBy(scroll) }
                                },
                            )
                        }
                    Column(dragModifier) { WontonGroup { TabSettingsRow(item, index, viewModel.items.size) } }
                }
                choices.filter { choice -> viewModel.items.none { it.tab == choice.tab } }.forEach { choice ->
                    item(key = "available:${choice.tab}") {
                        WontonGroup { TabPinSwitch(choice, checked = false, enabled = true) }
                    }
                }
            }
        }
    }
}

@Composable
private fun TabSettingsRow(item: TabItem, index: Int, count: Int) {
    val tabs = LocalTabs.current
    val title = item.tabTitle()
    Column {
        TabPinSwitch(TabChoice(item.tab, title), checked = true, enabled = item.removable)
        Row {
            TabMoveButton(title, tabText("上移", "Move up"), index > 0) {
                tabs.send(Action.MoveTab(item.tab, (index - 1).toUInt()))
            }
            TabMoveButton(title, tabText("下移", "Move down"), index < count - 1) {
                tabs.send(Action.MoveTab(item.tab, (index + 1).toUInt()))
            }
        }
    }
}

@Composable
private fun TabMoveButton(title: String, direction: String, enabled: Boolean, onClick: () -> Unit) {
    TextButton(onClick, enabled = enabled, modifier = Modifier.heightIn(min = 48.dp)
        .semantics { contentDescription = "$direction $title" }) {
        Text(direction, style = WontonTheme.typography.supporting)
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

