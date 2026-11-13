package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import app.wonton.protocol.AppearanceViewModel
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PanelItem
import app.wonton.protocol.PanelScope
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.roundToInt

private val DOT = 7.dp
private val DOT_OFFSET = 4.dp
private val DOT_GAP = 8.dp
private const val FLIP_PROJECT = 0.12f
private const val PAGE_MATCH = 0.01f
private const val PAGE_WINDOW = 1.01f

/**
 * 聊天页和从列表那一头拉出来的面板（ADR-0052）：面板垫在聊天页后面，聊天页跟着手指移动。
 * 主聊天页的面板在上（往下拉），房间的面板在下（往上推）。壳只画核心给的面板数据。
 */
@Suppress("CyclomaticComplexMethod") // 层叠、返回、几何都在这，拆开要带一堆参数。
@Composable
internal fun ChatPanelHost(
    panel: ChatPanelState,
    tabs: PagesState,
    home: HomeViewModel,
    appearance: AppearanceViewModel,
    onDraft: (String) -> Unit,
    onDetails: (String) -> Unit,
    content: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    val status = WindowInsets.statusBars.asPaddingValues()
    val navigation = WindowInsets.navigationBars.asPaddingValues()
    DisposableEffect(panel) {
        panel.onBegin = {
            keyboard?.hide()
            focus.clearFocus()
        }
        onDispose { panel.onBegin = null }
    }
    LaunchedEffect(panel.visible, panel.selection) {
        if (panel.visible) {
            panel.selection?.let { tabs.send(app.wonton.protocol.Action.SelectPanelPage(panel.scope, it)) }
        }
    }
    LaunchedEffect(panel.pages.isEmpty()) {
        if (panel.pages.isEmpty() && panel.level == PanelLevel.FULL) panel.move(PanelLevel.PEEK)
    }
    val current = panel.controller
    BoxWithConstraints(Modifier.fillMaxSize()) {
        panel.ready(constraints.maxHeight.toFloat(), with(density) { status.calculateTopPadding().toPx() },
            with(density) { navigation.calculateBottomPadding().toPx() })
        panel.geometry(density.density)
        val height = constraints.maxHeight.toFloat()
        val shift = panel.shiftPx
        val safeBottom = with(density) { navigation.calculateBottomPadding() }
        if (panel.visible) {
            Box(
                Modifier.fillMaxSize().graphicsLayer {
                    translationY = if (panel.edge == PanelEdge.BOTTOM) height - shift else 0f
                },
            ) {
                PanelPager(panel, tabs, home, appearance, onDraft, onDetails, Modifier.fillMaxSize())
                panel.controller?.customView?.let { video ->
                    androidx.compose.ui.viewinterop.AndroidView(
                        factory = { video }, modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        Box(Modifier.fillMaxSize().graphicsLayer { translationY = shift * panel.sign }) {
            content()
            if (panel.level == PanelLevel.PEEK) ChatPanelStrip(panel)
        }
        ChatPanelEdge(panel, panel.visible)
        ChatPanelTopPull(panel)
        if (!panel.visible && panel.pages.any { it.attention }) {
            Box(
                Modifier.align(if (panel.edge == PanelEdge.TOP) Alignment.TopCenter else Alignment.BottomCenter)
                    .padding(top = if (panel.edge == PanelEdge.TOP) status.calculateTopPadding() + DOT_OFFSET
                    else 0.dp, bottom = if (panel.edge == PanelEdge.TOP) 0.dp else safeBottom + DOT_OFFSET)
                    .size(DOT).background(WontonTheme.colors.danger.toColor(), CircleShape),
            )
        }
    }
    // 组合在内容之后：后注册的返回处理器先拿到返回键，面板开着时先关面板，不关房间。
    BackHandler(panel.level != PanelLevel.CLOSED) { panel.back() }
}

@Composable
private fun PanelPager(
    panel: ChatPanelState,
    tabs: PagesState,
    home: HomeViewModel,
    appearance: AppearanceViewModel,
    onDraft: (String) -> Unit,
    onDetails: (String) -> Unit,
    modifier: Modifier,
) {
    val pages = panel.pages
    if (pages.isEmpty()) {
        PanelEmpty { panel.editing = true }
        return
    }
    BoxWithConstraints(modifier.clipToBounds()) {
        val width = constraints.maxWidth.toFloat()
        val chosen = pages.indexOfFirst { it.tab == panel.selection }.coerceAtLeast(0)
        var position by remember(pages.size) { mutableFloatStateOf(chosen.toFloat()) }
        var motion by remember { mutableStateOf<Job?>(null) }
        val scope = rememberCoroutineScope()
        // 翻页的回调只建一次：面板页的触摸观察者按它认实例，重组换新 lambda 会打断拖动中的手势。
        val settlePage: (Float) -> Unit = remember {
            { raw ->
                motion?.cancel()
                val page = raw.roundToInt().coerceIn(0, panel.pages.lastIndex)
                val landed = panel.pages.getOrNull(page)?.tab
                if (abs(page - position) > PAGE_MATCH || panel.selection != landed) {
                    motion = scope.launch {
                        animate(
                            position, page.toFloat(),
                            animationSpec = spring<Float>(stiffness = Spring.StiffnessMediumLow),
                        ) { value, _ -> position = value }
                        landed?.let(panel::select)
                    }
                }
            }
        }
        val flipTo: (Float) -> Unit = remember {
            { target ->
                motion?.cancel()
                position = target.coerceIn(0f, panel.pages.lastIndex.toFloat())
            }
        }
        LaunchedEffect(chosen) {
            if (abs(chosen - position) > PAGE_MATCH) settlePage(chosen.toFloat())
        }
        val touch = remember { PanelPageTouch(panel) }
        SideEffect {
            touch.position = { position }
            touch.width = { width }
            touch.flipTo = flipTo
            touch.flipEnd = { velocity -> settlePage(position - velocity / width * FLIP_PROJECT) }
        }
        Box(Modifier.fillMaxSize()) {
            pages.forEachIndexed { index, item ->
                if (abs(index - position) <= PAGE_WINDOW) {
                    key(item.tab) {
                        PanelPage(
                            item = item,
                            scope = panel.scope,
                            panel = panel,
                            tabs = tabs,
                            home = home,
                            appearance = appearance,
                            active = panel.visible,
                            current = index == chosen,
                            touch = touch,
                            modifier = Modifier.fillMaxSize().graphicsLayer {
                                translationX = (index - position) * width
                            },
                            onDraft = onDraft,
                            onDetails = onDetails,
                        )
                    }
                }
            }
        }
        if (pages.size > 1) {
            PanelDots(panel = panel, pages = pages, onPick = { picked: TabRef ->
                panel.select(picked)
                settlePage(pages.indexOfFirst { it.tab == picked }.toFloat())
            })
        }
    }
}

/** 第几页的小圆点：第 1 级贴着露出的那条，第 2 级在 Home 条上方。要关注的页带红点。 */
@Composable
private fun BoxScope.PanelDots(panel: ChatPanelState, pages: List<PanelItem>, onPick: (TabRef) -> Unit) {
    val colors = WontonTheme.colors
    val safeBottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val where = when {
        panel.level == PanelLevel.FULL ->
            Modifier.align(Alignment.BottomCenter).padding(bottom = safeBottom + WontonTheme.spacing.s32)
        panel.edge == PanelEdge.TOP ->
            Modifier.align(Alignment.BottomCenter).padding(
                bottom = with(LocalDensity.current) { (panel.height - panel.shiftPx).toDp() } +
                    WontonTheme.spacing.s8,
            )
        else -> Modifier.align(Alignment.TopCenter).padding(top = WontonTheme.spacing.s8)
    }
    Box(Modifier.fillMaxSize()) {
        Box(
            where.background(colors.surface.toColor().copy(alpha = 0.9f), RoundedCornerShape(WontonTheme.radii.pill))
                .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s8),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(DOT_GAP)) {
                pages.forEach { item ->
                    Box(
                        Modifier.size(DOT)
                            .background(
                                if (item.tab == panel.selection) {
                                    colors.textPrimary.toColor()
                                } else {
                                    colors.textPrimary.toColor().copy(alpha = 0.3f)
                                },
                                CircleShape,
                            )
                            .clickable(role = Role.Button) { onPick(item.tab) }
                            .semantics { contentDescription = item.page.title },
                    ) {
                        if (item.attention) {
                            Box(
                                Modifier.align(Alignment.TopEnd).offset(x = DOT_OFFSET, y = -DOT_OFFSET)
                                    .size(DOT).background(colors.danger.toColor(), CircleShape),
                            )
                        }
                    }
                }
            }
        }
    }
}

/** 空面板：中间一个大加号，点了编辑面板。没有页面就没有胶囊，最多到第 1 级。 */
@Composable
private fun PanelEmpty(onAdd: () -> Unit) {
    val label = stringResource(R.string.panel_add_pages)
    val colors = WontonTheme.colors
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            Box(
                Modifier.size(WontonTheme.size.detailIcon)
                    .background(colors.surface.toColor(), CircleShape)
                    .border(WontonTheme.size.hairline, colors.separator.toColor(), CircleShape)
                    .clickable(role = Role.Button, onClick = onAdd)
                    .semantics { contentDescription = label },
                contentAlignment = Alignment.Center,
            ) {
                Icon(WontonIcon.Add, null, Modifier.size(WontonTheme.size.target),
                    tint = colors.textPrimary.toColor())
            }
            Text(label, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        }
    }
}

/** 消息列表上的读屏动作“打开面板”：过度拉动做不了，用一个动作补上。 */
@Composable
internal fun Modifier.panelAction(panel: ChatPanelState?): Modifier {
    if (panel == null) return this
    val label = stringResource(R.string.panel_open)
    return semantics {
        customActions = listOf(CustomAccessibilityAction(label) {
            panel.move(PanelLevel.PEEK)
            true
        })
    }
}
