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
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.ui.R
import app.wonton.ui.icons.Back
import app.wonton.ui.icons.WontonIcon

private const val PRESSED_ALPHA = 0.6f
private const val DISABLED_ALPHA = 0.4f

enum class WontonButtonKind { PRIMARY, SECONDARY, DANGER }

class WontonBarItem(val label: String, val icon: ImageVector, val selectedIcon: ImageVector)

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

/** 单行输入框。 */
@Composable
fun WontonTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String? = null,
    label: String? = null,
    singleLine: Boolean = true,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
) {
    val colors = WontonTheme.colors
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        singleLine = singleLine,
        textStyle = WontonTheme.typography.body.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        keyboardOptions = keyboardOptions,
        keyboardActions = keyboardActions,
        modifier = modifier.semantics { label?.let { contentDescription = it } },
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

/** 根页面的标题栏：大标题在前，操作在后。 */
@Composable
fun WontonRootBar(title: String, actions: @Composable RowScope.() -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth()
            .statusBarsPadding()
            .heightIn(min = WontonTheme.size.topBar)
            .padding(horizontal = WontonTheme.spacing.pageInset),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(
            title,
            Modifier.weight(1f).semantics { heading() },
            style = WontonTheme.typography.screenTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        actions()
    }
}

/** 进入下一层的页面的标题栏：返回在前，标题居中，操作在后。 */
@Composable
fun WontonPushedBar(title: String, onBack: () -> Unit, actions: @Composable RowScope.() -> Unit = {}) {
    Box(
        Modifier.fillMaxWidth()
            .statusBarsPadding()
            .heightIn(min = WontonTheme.size.topBar)
            .padding(horizontal = WontonTheme.spacing.s4),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            title,
            Modifier.padding(horizontal = WontonTheme.size.target + WontonTheme.spacing.s8).semantics { heading() },
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            WontonIconButton(WontonIcon.Back, "返回", onBack)
            Box(Modifier.weight(1f))
            actions()
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
