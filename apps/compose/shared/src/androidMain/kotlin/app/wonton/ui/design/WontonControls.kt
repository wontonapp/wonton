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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.ui.R
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
        Text(text, style = WontonTheme.typography.body, color = content.toColor())
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
    Column(
        Modifier.fillMaxWidth().background(WontonTheme.colors.canvas.toColor())
            .statusBarsPadding().semantics { paneTitle = title },
    ) {
        if (statusTitle != null || actions != null) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.topBar)
                    .padding(horizontal = WontonTheme.spacing.pageInset),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.End,
            ) {
                if (statusTitle != null) {
                    Text(
                        statusTitle,
                        Modifier.weight(1f),
                        style = WontonTheme.typography.supporting,
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
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
 * 进入下一层的 Material 标题栏：返回在前，标题靠左，操作在后。
 *
 * `showsStatus` 把连接状态放进副标题；`showsStatusLine` 给内容全靠连接的页面：长时间连不上时换成标题下面那一行，带下一步，
 * 不然页面只会一直转圈。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WontonPushedBar(
    title: String,
    onBack: () -> Unit,
    icon: ImageVector = WontonIcon.Back,
    iconLabel: String = "返回",
    showsStatus: Boolean = false,
    showsStatusLine: Boolean = false,
    actions: @Composable RowScope.() -> Unit = {},
) {
    val status = LocalWontonBarStatus.current
    val line = status.line.takeIf { showsStatusLine }
    val subtitle = status.subtitle.takeIf { (showsStatus || showsStatusLine) && line == null }
    Column(Modifier.fillMaxWidth()) {
        TopAppBar(
            title = {
                Column {
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
            },
            navigationIcon = { WontonIconButton(icon, iconLabel, onBack) },
            actions = actions,
            colors = TopAppBarDefaults.topAppBarColors(containerColor = WontonTheme.colors.canvas.toColor()),
        )
        line?.let {
            Box(
                Modifier.padding(horizontal = WontonTheme.spacing.pageInset)
                    .padding(bottom = WontonTheme.spacing.s8),
            ) { it() }
        }
    }
}

/** 页面的底色和标题栏。内容占满标题栏以下的部分。 */
@Composable
fun WontonScreen(bar: @Composable () -> Unit, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Column(modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())) {
        bar()
        Box(Modifier.weight(1f).fillMaxWidth()) { content() }
    }
}
