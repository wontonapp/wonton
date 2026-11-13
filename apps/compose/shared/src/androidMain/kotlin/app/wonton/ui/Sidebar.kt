package app.wonton.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.SpringSpec
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.AnimationVector1D
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.Rgb
import app.wonton.protocol.TabRef
import app.wonton.protocol.ThemeColors
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Apps
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.ChatFill
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.List
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.Store
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/** 根页面左上角打开侧边栏的按钮；推开的页面没有（WontonRootBar 读它）。 */
val LocalWontonOpenSidebar = compositionLocalOf<(() -> Unit)?> { null }

private const val PANEL_RATIO = 0.82f

/** 主界面被推开时缩小的量，以左边缘为锚点。 */
private const val MAIN_SHRINK = 0.06f

/** 关着时面板藏在主界面底下的比例：从 -18% 滑到 0。 */
private const val PANEL_TUCKED = 0.18f
private const val PANEL_DIM = 0.3f
private const val SCRIM_ALPHA = 0.22f

private const val SPRING_DAMPING = 0.86f
private const val SPRING_STIFFNESS = 270f
private const val BADGE_MAX = 99

/** 边缘热区占屏高的比例：系统返回手势的排除区最多认 200dp，2400px 屏上 0.22 ≈ 528px（≈220dp，压到上限内）。 */
private const val EDGE_ZONE_RATIO = 0.22f
private val OPEN_RADIUS = 40.dp
private val MAIN_SHADOW = 16.dp

/** 主界面以左边缘中点为锚点缩小。 */
private val MAIN_ANCHOR = TransformOrigin(0f, 0.5f)
private val STATUS_GAP = 6.dp
private val STATUS_DOT = 7.dp
internal val ROW_ICON = 28.dp

/** 侧边栏要去的页面。 */
internal sealed interface SidebarDestination {
    data object Chat : SidebarDestination

    data object Store : SidebarDestination

    data object AllApps : SidebarDestination

    data object Settings : SidebarDestination

    data object NewChat : SidebarDestination

    data class Plugin(val entry: PluginEntry) : SidebarDestination
}

/** 侧边栏的跟手手势：位移和速度都在这里；松手按位置和速度决定开还是弹回。 */
private class SidebarGestures(private val slide: Animatable<Float, AnimationVector1D>) {
    private val tracker = VelocityTracker()

    var dragging by mutableStateOf(false)
        private set

    var dragPx by mutableFloatStateOf(0f)
        private set

    fun start() {
        tracker.resetTracking()
        dragging = true
        dragPx = 0f
    }

    fun drag(change: PointerInputChange, dragAmount: Float) {
        tracker.addPosition(change.uptimeMillis, change.position)
        dragPx += dragAmount
        change.consume()
    }

    fun velocityX(): Float = tracker.calculateVelocity().x

    fun progress(base: Float, panelWidthPx: Float): Float =
        (if (dragging) base + dragPx / panelWidthPx else slide.value).coerceIn(0f, 1f)

    /** 松手：`base` 是这段手势的起点进度，`isOpen` 是当前的开合状态。 */
    fun settle(
        open: Boolean,
        base: Float,
        isOpen: Boolean,
        panelWidthPx: Float,
        scope: CoroutineScope,
        springSpec: SpringSpec<Float>,
        reduceMotion: Boolean,
        onSetOpen: (Boolean) -> Unit,
    ) {
        val from = (base + dragPx / panelWidthPx).coerceIn(0f, 1f)
        dragging = false
        dragPx = 0f
        if (open == isOpen) {
            // 弹回原状态：状态没变，动画要自己发。
            scope.launch {
                slide.snapTo(from)
                if (reduceMotion) slide.snapTo(base) else slide.animateTo(base, springSpec)
            }
        } else {
            scope.launch { slide.snapTo(from) }
            onSetOpen(open)
        }
    }
}

/** 跟手的横向拖拽：base 是这一段手势的起点进度（关着 0，开着 1）。 */
private fun Modifier.sidebarDrag(
    gestures: SidebarGestures,
    base: Float,
    isOpen: Boolean,
    panelWidthPx: Float,
    density: Float,
    scope: CoroutineScope,
    springSpec: SpringSpec<Float>,
    reduceMotion: Boolean,
    onSetOpen: (Boolean) -> Unit,
): Modifier = pointerInput(base, density) {
    detectHorizontalDragGestures(
        onDragStart = { gestures.start() },
        onDragEnd = {
            val direction = if (isOpen) -1f else 1f
            val commit = commitsGesture(
                gestures.dragPx * direction, gestures.velocityX() * direction,
                WontonTheme.gesture.swipeDistance, density,
            )
            gestures.settle(
                if (commit) !isOpen else isOpen, base, isOpen, panelWidthPx, scope, springSpec, reduceMotion, onSetOpen,
            )
        },
        onDragCancel = {
            gestures.settle(isOpen, base, isOpen, panelWidthPx, scope, springSpec, reduceMotion, onSetOpen)
        },
    ) { change, dragAmount ->
        gestures.drag(change, dragAmount)
    }
}

/**
 * 学 ChatGPT 的“推开”：面板从左边进来，主界面整页推到右边、缩小、变圆角，只露出右边一条。
 * 跟手：松开时按位置和速度决定打开还是弹回去（iOS 原型 SidebarContainer 的行为）。
 * 点露出来的一条或往左滑收起；系统返回键的收起由调用方用 BackHandler 做。
 */
@Composable
internal fun SidebarContainer(
    isOpen: Boolean,
    edgeSwipe: Boolean,
    onSetOpen: (Boolean) -> Unit,
    panel: @Composable (width: Dp, interactive: Boolean) -> Unit,
    content: @Composable () -> Unit,
) {
    val colors = WontonTheme.colors
    val scope = rememberCoroutineScope()
    val slide = remember { Animatable(initialValue = 0f) }
    val gestures = remember(slide) { SidebarGestures(slide) }
    val springSpec = spring<Float>(dampingRatio = SPRING_DAMPING, stiffness = SPRING_STIFFNESS)
    val context = LocalContext.current
    val reduceMotion = remember(context) { context.animatorScale() == 0f }

    BoxWithConstraints(Modifier.fillMaxSize().background(colors.surfaceSubtle.toColor())) {
        val panelWidth = maxWidth * PANEL_RATIO
        val panelWidthPx = with(LocalDensity.current) { panelWidth.toPx() }
        val density = LocalDensity.current.density
        val base = if (isOpen) 1f else 0f
        val progress = gestures.progress(base, panelWidthPx)

        // 状态变化（导航、返回键）后的开合动画；拖拽结束的那条路先 snapTo 到手松开的位置，这里接上。
        LaunchedEffect(isOpen) {
            if (reduceMotion) slide.snapTo(if (isOpen) 1f else 0f)
            else slide.animateTo(if (isOpen) 1f else 0f, springSpec)
        }

        // 面板：关着时缩在主界面底下，滑出来时从淡到实。
        Box(
            Modifier.align(Alignment.CenterStart)
                .fillMaxHeight()
                .width(panelWidth)
                .graphicsLayer {
                    alpha = PANEL_DIM + (1f - PANEL_DIM) * progress
                    translationX = -panelWidthPx * PANEL_TUCKED * (1f - progress)
                },
        ) {
            panel(panelWidth, progress > PANEL_INTERACTIVE_AT)
        }

        // 主界面：推到右边、缩小、圆角加阴影，露出的一条盖着暗色。
        Box(
            Modifier.fillMaxSize().graphicsLayer {
                translationX = panelWidthPx * progress
                val scale = 1f - MAIN_SHRINK * progress
                scaleX = scale
                scaleY = scale
                transformOrigin = MAIN_ANCHOR
                shape = RoundedCornerShape(OPEN_RADIUS * progress)
                clip = progress > 0f
                shadowElevation = MAIN_SHADOW.toPx() * progress
            },
        ) {
            content()
            if (progress > 0f) {
                val collapse = stringResource(R.string.sidebar_collapse)
                Box(
                    Modifier.matchParentSize()
                        .testTag("sidebar-close")
                        .background(Color.Black.copy(alpha = SCRIM_ALPHA * progress))
                        .then(
                            if (isOpen) Modifier.sidebarDrag(
                                gestures, base, isOpen, panelWidthPx, density,
                                scope, springSpec, reduceMotion, onSetOpen,
                            ) else Modifier,
                        )
                        .clickable(
                            remember { MutableInteractionSource() },
                            indication = null,
                            onClick = { onSetOpen(false) },
                        )
                        .semantics { contentDescription = collapse },
                )
            }
        }

        // 聊天根页面才有的左边缘右滑；进了下一层，左边缘留给系统的返回手势。
        // 系统返回手势占着左边缘，排除区最多认 200dp：热区是一条 200dp 高的条带，条带外的边缘仍是系统返回。
        if (edgeSwipe && !isOpen) {
            Box(
                Modifier.align(Alignment.CenterStart)
                    .fillMaxHeight(EDGE_ZONE_RATIO)
                    .width(WontonTheme.gesture.edgeWidth.dp)
                    .systemGestureExclusion()
                    .sidebarDrag(
                        gestures, base, isOpen, panelWidthPx, density, scope, springSpec, reduceMotion, onSetOpen,
                    ),
            )
        }
    }
}

/** 松手前就点不动面板里的东西的进度线，学 iOS 原型的 allowsHitTesting(progress > 0.5)。 */
private const val PANEL_INTERACTIVE_AT = 0.5f

/** 根页面左上角的侧边栏按钮（WontonRootBar 用）。 */
@Composable
internal fun SidebarMenuButton(onClick: () -> Unit) {
    val label = stringResource(R.string.sidebar_open)
    Box(
        Modifier.size(WontonTheme.size.target)
            .testTag("sidebar-open")
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            WontonIcon.List,
            // 装饰：按钮外层已经带了名字。
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textPrimary.toColor(),
        )
    }
}

/**
 * 侧边栏的内容：标题和连接状态、聊天、应用商店、我的应用，底下是设置和新聊天。
 */
@Composable
internal fun SidebarPanel(
    home: HomeViewModel,
    plugins: PluginsState,
    tabs: PagesState,
    current: TabRef,
    width: Dp,
    interactive: Boolean,
    onGo: (SidebarDestination) -> Unit,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    Column(
        Modifier.width(width).fillMaxHeight()
            .statusBarsPadding()
            .navigationBarsPadding(),
    ) {
        SidebarHeader(home)
        Column(
            Modifier.weight(1f)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = spacing.s12),
            verticalArrangement = Arrangement.spacedBy(spacing.s4),
        ) {
            SidebarRow(
                title = stringResource(R.string.sidebar_chats),
                selected = current == TabRef.Chat,
                interactive = interactive,
                onClick = { onGo(SidebarDestination.Chat) },
                icon = {
                    Icon(
                        if (current == TabRef.Chat) WontonIcon.ChatFill else WontonIcon.Chat,
                        // 装饰：行的文字在右边。
                        contentDescription = null,
                        Modifier.size(ROW_ICON),
                        tint = colors.textPrimary.toColor(),
                    )
                },
            )
            SidebarRow(
                title = stringResource(R.string.sidebar_store),
                selected = false,
                interactive = interactive,
                badge = plugins.viewModel.sidebar.updates.toInt(),
                onClick = { onGo(SidebarDestination.Store) },
                icon = {
                    Icon(
                        WontonIcon.Store,
                        // 装饰：行的文字在右边。
                        contentDescription = null,
                        Modifier.size(ROW_ICON),
                        tint = colors.textPrimary.toColor(),
                    )
                },
            )
            SidebarApps(plugins, tabs, current, interactive, onGo)
        }
        SidebarFooter(plugins, interactive, onGo)
    }
}

/** 顶部：应用名加一行服务器连接状态。 */
@Composable
private fun SidebarHeader(home: HomeViewModel) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    val status = home.status
    Column(
        Modifier.padding(
            start = spacing.s24, end = spacing.s24, top = spacing.s16, bottom = spacing.s24,
        ),
    ) {
        Text(
            stringResource(R.string.sidebar_title),
            style = WontonTheme.typography.screenTitle,
            color = colors.textPrimary.toColor(),
        )
        Spacer(Modifier.height(spacing.s4))
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(STATUS_GAP),
        ) {
            Box(Modifier.size(STATUS_DOT).background(statusColor(status, colors).toColor(), CircleShape))
            Text(
                statusText(status),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
    }
}

/** “我的应用”：用户选出来的行，核心排好序；装了还没加进的收进最后一行“全部应用”。 */
@Composable
internal fun SidebarApps(
    plugins: PluginsState,
    tabs: PagesState,
    current: TabRef,
    interactive: Boolean,
    onGo: (SidebarDestination) -> Unit,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    Row(
        Modifier.padding(start = spacing.s12, end = spacing.s12, top = spacing.s24, bottom = spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            stringResource(R.string.sidebar_my_apps),
            style = WontonTheme.typography.supporting.copy(fontWeight = FontWeight.SemiBold),
            color = colors.textSecondary.toColor(),
        )
        Spacer(Modifier.weight(1f))
        Text(
            stringResource(R.string.sidebar_all),
            Modifier.testTag("sidebar-all-apps").heightIn(min = WontonTheme.size.target).clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                enabled = interactive,
                onClick = { onGo(SidebarDestination.AllApps) },
            ),
            style = WontonTheme.typography.supporting,
            color = colors.accentInk.toColor(),
        )
    }
    // “我的应用”由核心给：顺序和选择在账号数据里，行尾红点也是核心算的。
    val apps = plugins.viewModel.sidebar.apps
    apps.forEach { app ->
        SidebarAppRow(app, plugins, tabs, current, interactive, onGo)
    }
    MyAppsSyncHint(tabs, interactive)
    if (plugins.viewModel.sidebar.moreApps) {
        SidebarRow(
            title = stringResource(R.string.sidebar_all_apps),
            selected = current == TabRef.Apps,
            interactive = interactive,
            onClick = { onGo(SidebarDestination.AllApps) },
            icon = {
                Icon(
                    WontonIcon.Apps,
                    // 装饰：行的文字在右边。
                    contentDescription = null,
                    Modifier.size(ROW_ICON),
                    tint = colors.textPrimary.toColor(),
                )
            },
        )
    }
}

/** 底部：左边圆形的设置按钮（有要处理的事时带红点），右边主色的“新聊天”胶囊。 */
@Composable
private fun SidebarFooter(
    plugins: PluginsState,
    interactive: Boolean,
    onGo: (SidebarDestination) -> Unit,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    Row(
        Modifier.padding(horizontal = spacing.s16, vertical = spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val settingsLabel = stringResource(R.string.sidebar_settings)
        val attentionLabel = stringResource(R.string.sidebar_needs_attention)
        Box(
            Modifier.size(WontonTheme.size.target)
                .testTag("sidebar-settings")
                .background(colors.surface.toColor(), CircleShape)
                .border(WontonTheme.size.hairline, colors.separator.toColor(), CircleShape)
                .clickable(
                    enabled = interactive,
                    role = Role.Button,
                    onClick = { onGo(SidebarDestination.Settings) },
                )
                .semantics { contentDescription = settingsLabel },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                WontonIcon.Settings,
                // 装饰：按钮外层已经带了名字。
                contentDescription = null,
                Modifier.size(WontonTheme.size.icon),
                tint = colors.textPrimary.toColor(),
            )
            if (plugins.viewModel.sidebar.settingsBadge) {
                Box(
                    Modifier.align(Alignment.TopEnd)
                        .size(UPDATE_DOT)
                        .background(colors.danger.toColor(), CircleShape)
                        .semantics { contentDescription = attentionLabel },
                )
            }
        }
        Spacer(Modifier.weight(1f))
        Row(
            Modifier.heightIn(min = WontonTheme.size.target)
                .clip(RoundedCornerShape(WontonTheme.radii.pill))
                .background(colors.primary.toColor())
                .clickable(
                    enabled = interactive,
                    role = Role.Button,
                    onClick = { onGo(SidebarDestination.NewChat) },
                )
                .padding(horizontal = spacing.s24),
            horizontalArrangement = Arrangement.spacedBy(spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                WontonIcon.Compose,
                // 装饰：按钮里有文字。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.onPrimary.toColor(),
            )
            Text(
                stringResource(R.string.sidebar_new_chat),
                style = WontonTheme.typography.itemTitle,
                color = colors.onPrimary.toColor(),
            )
        }
    }
}

@Composable
private fun statusText(status: HomeStatus): String = when (status) {
    HomeStatus.Connected -> stringResource(R.string.sidebar_status_connected)
    HomeStatus.Connecting -> stringResource(R.string.sidebar_status_connecting)
    is HomeStatus.Disconnected -> stringResource(R.string.sidebar_status_disconnected)
    else -> stringResource(R.string.sidebar_status_not_configured)
}

private fun statusColor(status: HomeStatus, colors: ThemeColors): Rgb = when (status) {
    HomeStatus.Connected -> colors.success
    HomeStatus.Connecting -> colors.warning
    else -> colors.danger
}

private fun Context.animatorScale(): Float =
    Settings.Global.getFloat(contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
