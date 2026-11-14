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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import app.wonton.protocol.PageAction
import app.wonton.protocol.PanelScope
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.zIndex
import app.wonton.protocol.Action
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.SidebarApp

import app.wonton.protocol.TabCandidate
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

private const val DRAG_HANDLE_LINES = 3
private const val DRAG_HANDLE_STROKE_SCALE = 1f / 12f
private const val DRAG_HANDLE_INSET = 1f / 6f

/** 一行"我的应用"对应的存储引用：页面行按页面，插件行按插件（Bot 私聊行）。 */
private fun SidebarApp.myAppRef(): MyAppRef? {
    val tab = this.tab
    val pluginId = this.pluginId
    return when {
        tab != null -> MyAppRef.Page(tab)
        pluginId != null -> MyAppRef.Plugin(pluginId)
        else -> null
    }
}

@Composable
fun MyAppsSettingsScreen(onBack: () -> Unit) {
    val pages = LocalPages.current
    var adding by remember { mutableStateOf(false) }
    var editingPanel by remember { mutableStateOf(false) }
    val editor = if (editingPanel) rememberChatPanelState(PanelEdge.TOP, PanelScope.Main) else null
    if (editor != null) {
        SideEffect { editor.feed(pages.viewModel.panel, pages.viewModel.panelSelected) }
        ChatPanelEditor(editor, pages) { editingPanel = false }
    }
    if (adding) {
        MyAppCandidatesScreen { adding = false }
        return
    }
    BackHandler(onBack = onBack)
    val list = rememberLazyListState()
    WontonScreen(bar = { MyAppsBar(stringResource(R.string.settings_my_apps), onBack) }) {
        LazyColumn(
            state = list,
            modifier = Modifier.fillMaxSize().padding(horizontal = WontonTheme.spacing.pageInset),
            contentPadding = wontonContentPaddingValues(),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            item(key = "auto") { WontonGroup { AutoAddRow(pages.viewModel.autoAdd) } }
            item(key = "current") {
                CurrentAppsGroup(pages.plugins.viewModel.sidebar.apps, list)
            }
            if (pages.viewModel.myAppCandidates.isNotEmpty()) {

                item(key = "add") {
                    WontonGroup {
                        WontonRow(
                            stringResource(R.string.myapp_add_more), showsForward = true, onClick = { adding = true },
                        )
                    }
                }
            }
            item(key = "problem") { PagesConfigurationProblem(pages.viewModel) }
        }
    }
}

/** 新装的应用默认自动加进"我的应用"末尾；这里可以关。 */
@Composable
private fun AutoAddRow(autoAdd: Boolean) {
    val pages = LocalPages.current
    val title = stringResource(R.string.myapp_auto_add)
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(title, Modifier.weight(1f), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor())
        Box(Modifier.semantics { contentDescription = title }) {
            WontonSwitch(autoAdd, { pages.send(Action.Pages(PageAction.SetAutoAddApps(it))) })
        }
    }
}

/** Only the rows inside the group participate in drag destinations, not the lazy list's other items. */
@Composable
private fun CurrentAppsGroup(apps: List<SidebarApp>, list: LazyListState) {
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    val edgeSize = WontonTheme.size.target
    val heights = remember { mutableMapOf<MyAppRef, Int>() }
    var dragging by remember(apps) { mutableStateOf<MyAppRef?>(null) }
    var displacement by remember { mutableStateOf(0f) }
    var pointerY by remember { mutableStateOf(0f) }
    var destination by remember { mutableStateOf(0) }
    fun locateDestination() {
        var bottom = 0
        destination = apps.indexOfFirst { app ->
            bottom += heights[app.myAppRef()] ?: 0
            pointerY < bottom
        }.let { if (it < 0) apps.lastIndex else it }
    }
    WontonGroup {
        apps.forEachIndexed { index, app ->
            val appRef = app.myAppRef() ?: return@forEachIndexed
            val modifier = Modifier.onSizeChanged { heights[appRef] = it.height }
                .zIndex(if (dragging == appRef) 1f else 0f)
                .graphicsLayer { translationY = if (dragging == appRef) displacement else 0f }
                .pointerInput(apps) {
                    detectDragGesturesAfterLongPress(
                        onDragStart = { offset ->
                            dragging = appRef
                            displacement = 0f
                            pointerY = (0 until index).sumOf { heights[apps[it].myAppRef()] ?: 0 } + offset.y
                            destination = index
                        },
                        onDragCancel = { dragging = null },
                        onDragEnd = {
                            if (destination != index) {
                                pages.send(Action.Pages(PageAction.MoveMyApp(appRef, destination.toUInt())))
                            }
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
                                if (dragging == appRef) {
                                    displacement += consumed
                                    pointerY += consumed
                                    locateDestination()
                                }
                            }
                        },
                    )
                }
            key(appRef) { MyAppSettingsRow(pages, app, appRef, index, apps.size, modifier) }
        }
    }
}

@Composable
private fun MyAppSettingsRow(
    pages: PagesState, app: SidebarApp, appRef: MyAppRef, index: Int, count: Int, modifier: Modifier,
) {
    val moveUp = stringResource(R.string.tab_move_up)
    val moveDown = stringResource(R.string.tab_move_down)
    val colors = WontonTheme.colors
    Row(
        modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .semantics(mergeDescendants = true) {
                customActions = buildList {
                    if (index > 0) add(CustomAccessibilityAction(moveUp) {
                        pages.send(Action.Pages(PageAction.MoveMyApp(appRef, (index - 1).toUInt()))); true
                    })
                    if (index < count - 1) add(CustomAccessibilityAction(moveDown) {
                        pages.send(Action.Pages(PageAction.MoveMyApp(appRef, (index + 1).toUInt()))); true
                    })
                }
            }.padding(start = WontonTheme.spacing.s16, end = WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        TabDragHandle()
        if (app.pluginId != null) {
            PluginIconImage(app.icon, WontonTheme.size.icon, pages.plugins)
        } else {
            Icon(WontonIcon.Settings, null, Modifier.size(WontonTheme.size.icon),
                tint = WontonTheme.colors.textPrimary.toColor())
        }
        Text(app.title, Modifier.weight(1f).padding(vertical = WontonTheme.spacing.s12),
            style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        WontonIconButton(WontonIcon.Trash, stringResource(R.string.myapp_remove),
            { pages.send(Action.Pages(PageAction.RemoveMyApp(appRef))) }, tint = colors.danger.toColor())
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
private fun MyAppCandidatesScreen(onBack: () -> Unit) {
    val pages = LocalPages.current
    BackHandler(onBack = onBack)
    WontonScreen(bar = { MyAppsBar(stringResource(R.string.myapp_add), onBack) }) {
        LazyColumn(
            Modifier.fillMaxSize().padding(horizontal = WontonTheme.spacing.pageInset),
            contentPadding = wontonContentPaddingValues(),
        ) {
            items(pages.viewModel.myAppCandidates, key = { it.tab.toString() }) { candidate ->
                WontonGroup { MyAppCandidateRow(candidate) }
            }
            item { PagesConfigurationProblem(pages.viewModel) }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MyAppsBar(title: String, onBack: () -> Unit) {
    WontonPushedBar(title, onBack)
}

@Composable
private fun MyAppCandidateRow(candidate: TabCandidate) {
    val pages = LocalPages.current
    val plugin = candidate.plugin
    val title = plugin?.pageTitle?.let { "${plugin.title} · $it" } ?: plugin?.title
        ?: stringResource(R.string.tab_settings)
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button) {
                pages.send(Action.Pages(PageAction.AddMyApp(MyAppRef.Page(candidate.tab), null, null)))
            }
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        // 装饰：图标旁边就是行标题。
        if (plugin != null) PluginIconImage(plugin.icon, WontonTheme.size.icon, pages.plugins)
        else Icon(WontonIcon.Settings, null, Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor())
        Text(title, Modifier.weight(1f), style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor())
        // 装饰：整行可点，加号只表示点了会加进去。
        Icon(WontonIcon.Add, null, Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor())
    }
}

@Composable
internal fun PagesConfigurationProblem(viewModel: app.wonton.protocol.PagesViewModel) {
    viewModel.problem?.let {
        ProblemNotice(Problem(stringResource(R.string.myapp_problem), detail = it.message))
    }
}
