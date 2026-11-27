package app.wonton.ui.design

import android.graphics.BitmapFactory
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageShader
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.Measurable
import androidx.compose.ui.layout.MeasurePolicy
import androidx.compose.ui.layout.MeasureResult
import androidx.compose.ui.layout.MeasureScope
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import app.wonton.ui.R
import app.wonton.ui.LocalWontonOpenSidebar
import app.wonton.ui.SidebarMenuButton
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.WontonIcon

private const val PRESSED_ALPHA = 0.6f
private const val DISABLED_ALPHA = 0.4f

enum class WontonButtonKind { PRIMARY, SECONDARY, DANGER }

/** 弹窗里的一个按钮。 */
class WontonDialogAction(
    val title: String,
    val kind: WontonButtonKind = WontonButtonKind.SECONDARY,
    val onClick: () -> Unit,
)

/** 底部面板里的一行操作。 */
class WontonSheetAction(
    val title: String,
    val icon: ImageVector,
    val isDanger: Boolean = false,
    val onClick: () -> Unit,
)

/** 自己画的按钮（ADR-0026）：语义还是 `clickable` 的按钮，命中区 48。 */
@Composable
fun WontonButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: WontonButtonKind = WontonButtonKind.PRIMARY,
    enabled: Boolean = true,
    icon: ImageVector? = null,
) {
    val colors = WontonTheme.colors
    val (fill, content) = when (kind) {
        WontonButtonKind.PRIMARY -> colors.primary to colors.onPrimary
        WontonButtonKind.SECONDARY -> colors.surfaceSubtle to colors.textPrimary
        WontonButtonKind.DANGER -> colors.surfaceSubtle to colors.danger
    }
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    Box(
        modifier.heightIn(min = WontonTheme.size.target)
            .alpha(if (!enabled) DISABLED_ALPHA else if (pressed) PRESSED_ALPHA else 1f)
            .clip(shape)
            .background(fill.toColor())
            .clickable(interaction, indication = null, enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = WontonTheme.spacing.s16),
        contentAlignment = Alignment.Center,
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            icon?.let {
                Icon(it, contentDescription = null, Modifier.size(WontonTheme.size.iconSmall), tint = content.toColor())
            }
            Text(text, style = WontonTheme.typography.body, color = content.toColor())
        }
    }
}

/** 图标按钮：图形 24，命中区 48。 */
@Composable
fun WontonIconButton(icon: ImageVector, label: String, onClick: () -> Unit, tint: Color? = null) {
    Box(
        Modifier.size(WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            icon,
            // 装饰：外层 WontonIconButton 已经带了名字。
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = tint ?: WontonTheme.colors.textPrimary.toColor(),
        )
    }
}

/** 输入框，`maxLines` 大于 1 时随内容长高。 */
@Composable
fun WontonTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String? = null,
    label: String? = null,
    textStyle: TextStyle = WontonTheme.typography.body,
    maxLines: Int = 1,
    enabled: Boolean = true,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
) {
    val colors = WontonTheme.colors
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        enabled = enabled,
        singleLine = maxLines == 1,
        maxLines = maxLines,
        textStyle = textStyle.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        visualTransformation = visualTransformation,
        keyboardOptions = keyboardOptions,
        keyboardActions = keyboardActions,
        modifier = modifier
            .alpha(if (enabled) 1f else DISABLED_ALPHA)
            .semantics { label?.let { contentDescription = it } },
        decorationBox = { field ->
            Box(
                Modifier.fillMaxWidth()
                    .heightIn(min = WontonTheme.size.target)
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                    .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s12),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (value.isEmpty() && placeholder != null) {
                    Text(placeholder, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
                }
                field()
            }
        },
    )
}

/** 提示块：错误、权限和不能用的原因，要做的事放在 `actions`。 */
@Composable
fun WontonNotice(
    icon: ImageVector,
    title: String,
    isError: Boolean = true,
    lines: List<String> = emptyList(),
    actions: @Composable () -> Unit = {},
) {
    val colors = WontonTheme.colors
    val titleColor = (if (isError) colors.danger else colors.textPrimary).toColor()
    Column(
        Modifier.fillMaxWidth()
            .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.card))
            .padding(WontonTheme.spacing.s12),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            // 装饰：标题就在右边。
            Icon(icon, contentDescription = null, Modifier.size(WontonTheme.size.iconSmall), tint = titleColor)
            Text(title, style = WontonTheme.typography.body, color = titleColor)
        }
        lines.forEach { line ->
            Text(line, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        }
        actions()
    }
}

/** 噪点贴图平铺，1 个噪点像素对 1 个物理像素（ADR-0026）。按 dp 平铺时颗粒会随屏幕密度放大变糊。 */
@Composable
fun Modifier.noise(shape: Shape): Modifier {
    val resources = LocalContext.current.resources
    val brush = remember(resources) {
        val options = BitmapFactory.Options().apply { inScaled = false }
        val bitmap = BitmapFactory.decodeResource(resources, R.drawable.wonton_noise, options).asImageBitmap()
        ShaderBrush(ImageShader(bitmap, TileMode.Repeated, TileMode.Repeated))
    }
    return drawNoise(brush, shape)
}

private fun Modifier.drawNoise(brush: Brush, shape: Shape) = drawWithCache {
    val outline = shape.createOutline(size, layoutDirection, this)
    onDrawWithContent {
        drawOutline(outline, brush)
        drawContent()
    }
}

/** 重点色的填充：纸质的颗粒盖在颜色上，有描边令牌时加描边。 */
@Composable
fun Modifier.accentFill(shape: Shape, border: Boolean = true): Modifier {
    val colors = WontonTheme.colors
    val stroke = colors.accentFillBorder?.takeIf { border }
    return background(colors.accentFill.toColor(), shape)
        .noise(shape)
        .then(
            if (stroke != null) Modifier.border(WontonTheme.size.hairline, stroke.toColor(), shape) else Modifier,
        )
}

/** 根页面只显示连接状态和操作，页面标题留给读屏。 */
@Composable
fun WontonRootBar(title: String, actions: (@Composable RowScope.() -> Unit)? = null) {
    val status = LocalWontonBarStatus.current
    val statusTitle = status.title?.takeIf { it.isNotBlank() }
    val openSidebar = LocalWontonOpenSidebar.current
    Column(
        Modifier.fillMaxWidth()
            .statusBarsPadding().semantics { paneTitle = title },
    ) {
        if (statusTitle != null || actions != null || openSidebar != null) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.topBar)
                    .padding(horizontal = WontonTheme.spacing.pageInset),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.End,
            ) {
                openSidebar?.let { SidebarMenuButton(it) }
                if (statusTitle != null) {
                    Text(
                        statusTitle,
                        Modifier.weight(1f),
                        style = WontonTheme.typography.supporting,
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                } else {
                    Spacer(Modifier.weight(1f))
                }
                actions?.invoke(this)
            }
        }
        status.line?.let { line ->
            Box(
                Modifier.padding(horizontal = WontonTheme.spacing.pageInset)
                    .padding(bottom = WontonTheme.spacing.s8),
            ) { line() }
        }
    }
}

/** 栏里的连接状态：短文本、聊天房间副标题和需要下一步操作的长提示。 */
class WontonBarStatus(
    val title: String? = null,
    val subtitle: String? = null,
    val line: (@Composable () -> Unit)? = null,
)

val LocalWontonBarStatus = compositionLocalOf { WontonBarStatus() }

/**
 * 进入下一层的标题栏：返回在前，标题靠左，操作在后。
 *
 * 用自定义 Layout 而不是 M3 的 TopAppBar：顶部栏的子树带 pointer input，命中测试停在栏里就不再下探，
 * 滚进栏下的内容看得见、点不到（#688）；栏本身只是透明背景，点击会落到 [WontonScreen] 的内容上。
 *
 * `centeredTitle` 把标题挪到正中（群聊用它，两边不放头像）；带 `avatar` 的标题只能在左边。
 * `showsStatus` 把连接状态放进副标题；`showsStatusLine` 给内容全靠连接的页面：长时间连不上时换成标题下面那一行，带下一步，
 * 不然页面只会一直转圈。`avatar` 放在标题前（和 Bot 私聊的房间用它显示状态头像，ADR-0044）。
 * 给了 `avatarClick` 时头像能点（私聊点进 Bot 资料页），`avatarLabel` 是头像的读屏名；标题只显示，不做第二个入口。
 */
@Composable
fun WontonPushedBar(
    title: String,
    onBack: () -> Unit,
    icon: ImageVector = WontonIcon.Back,
    iconLabel: String = stringResource(R.string.design_back),
    showsStatus: Boolean = false,
    showsStatusLine: Boolean = false,
    avatar: (@Composable () -> Unit)? = null,
    avatarClick: (() -> Unit)? = null,
    avatarLabel: String = "",
    centeredTitle: Boolean = false,
    actions: (@Composable RowScope.() -> Unit)? = null,
) {
    val status = LocalWontonBarStatus.current
    val line = status.line.takeIf { showsStatusLine }
    val subtitle = status.subtitle.takeIf { (showsStatus || showsStatusLine) && line == null }
    val titleSlot: @Composable () -> Unit = {
        Column(Modifier.testTag("roomTopBarTitle")) {
            Text(
                title,
                Modifier.semantics { heading() },
                style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor(),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            subtitle?.let {
                Text(
                    it,
                    style = WontonTheme.typography.caption,
                    color = WontonTheme.colors.textSecondary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
    // 居中沿用 M3 的做法：标题对整栏居中，再按两侧量到的宽度夹紧（先左后右），
    // 不能用 weight 盒加固定占位，否则 actions 的宽度一变标题就偏。
    val minHeight = WontonTheme.size.target + WontonTheme.spacing.s16
    Column(
        Modifier.fillMaxWidth()
            .windowInsetsPadding(
                WindowInsets.systemBars.only(WindowInsetsSides.Top + WindowInsetsSides.Horizontal),
            )
            .semantics {
                isTraversalGroup = true
                paneTitle = title
            },
    ) {
        Layout(
            content = {
                WontonIconButton(icon, iconLabel, onBack)
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                ) {
                    avatar?.let {
                        Box(
                            Modifier
                                .testTag("roomTopBarAvatar")
                                .then(
                                    if (avatarClick == null) {
                                        Modifier
                                    } else {
                                        Modifier.clickable(role = Role.Button, onClick = avatarClick)
                                    },
                                )
                                .semantics { contentDescription = avatarLabel },
                        ) {
                            it()
                        }
                    }
                    titleSlot()
                }
                Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
                    actions?.invoke(this)
                }
            },
            // M3 的栏高 64dp；用令牌拼出来，不写死在调用点上。
            modifier = Modifier.fillMaxWidth().heightIn(min = minHeight)
                .padding(horizontal = WontonTheme.spacing.pageInset),
            measurePolicy = PushedBarMeasurePolicy(
                minHeight, WontonTheme.spacing.s4, WontonTheme.spacing.s12, centeredTitle,
            ),
        )
        line?.let {
            Box(
                Modifier.padding(horizontal = WontonTheme.spacing.pageInset)
                    .padding(bottom = WontonTheme.spacing.s8),
            ) { it() }
        }
    }
}


/** WontonPushedBar 的量和摆：M3 `placeTopAppBar` 的居中，再按两侧实测宽度夹紧（先左后右）。 */
private class PushedBarMeasurePolicy(
    private val minBarHeight: Dp,
    private val titleInset: Dp,
    private val gap: Dp,
    private val centeredTitle: Boolean,
) : MeasurePolicy {
    override fun MeasureScope.measure(measurables: List<Measurable>, constraints: Constraints): MeasureResult {
        val loose = Constraints(
            minWidth = 0,
            maxWidth = constraints.maxWidth,
            minHeight = 0,
            maxHeight = constraints.maxHeight,
        )
        val back = measurables[0].measure(loose)
        val trailing = measurables[2].measure(loose)
        // 和原来 Row 的间距一致：返回键与标题、标题与 actions 各 12dp；标题最左至少是 M3 的 4dp 内缩。
        // 先算两侧让位再量标题：限宽和真正可用的跨度一致，长标题才不会压到返回键底下（夹紧也不会越界）。
        val gapPx = gap.roundToPx()
        val start = maxOf(titleInset.roundToPx(), back.width) + gapPx
        val end = trailing.width + gapPx
        val titleLimit = if (constraints.maxWidth == Constraints.Infinity) constraints.maxWidth
        else (constraints.maxWidth - start - end).coerceAtLeast(0)
        val title = measurables[1].measure(
            Constraints(minWidth = 0, maxWidth = titleLimit, minHeight = 0, maxHeight = constraints.maxHeight),
        )
        val contentHeight = maxOf(minBarHeight.roundToPx(), back.height, title.height, trailing.height)
        val barHeight = if (constraints.maxHeight == Constraints.Infinity) contentHeight
        else contentHeight.coerceAtMost(constraints.maxHeight)
        var titleX = if (centeredTitle) (constraints.maxWidth - title.width) / 2 else 0
        if (titleX < start) titleX = start
        val rightmost = constraints.maxWidth - end - title.width
        if (titleX > rightmost) titleX = rightmost
        return layout(constraints.maxWidth, barHeight) {
            back.placeRelative(0, (barHeight - back.height) / 2)
            title.placeRelative(titleX, (barHeight - title.height) / 2)
            trailing.placeRelative(constraints.maxWidth - trailing.width, (barHeight - trailing.height) / 2)
        }
    }
}
