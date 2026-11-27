package app.wonton.ui.design

import androidx.compose.foundation.Image
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer

/** 图标画彩色分层还是单色，整个 App 一个取值（ADR-0026）。单色是同一套图形，颜色跟 `tint`。 */
val LocalWontonIconMono = staticCompositionLocalOf { false }

/** 图上的一个字（如“帮助”的问号）：上游用系统字体写，ImageVector 没有文字，由 [Icon] 画。位置和大小是图标边长的比例。 */
class WontonGlyphLabel(
    val text: String,
    val centerX: Float,
    val baseline: Float,
    val size: Float,
    val bold: Boolean,
    /** 在颜色表里的下标。 */
    val color: Int,
)

/** 一个图标：同一份图形配浅色、深色、单色三套颜色，用到哪套才建哪套。 */
class WontonGlyph(
    private val name: String,
    private val light: LongArray,
    private val dark: LongArray,
    private val mono: LongArray,
    val label: WontonGlyphLabel? = null,
    private val build: (String, LongArray) -> ImageVector,
) {
    private val lightVector by lazy(LazyThreadSafetyMode.NONE) { build("${name}Light", light) }
    private val darkVector by lazy(LazyThreadSafetyMode.NONE) { build("${name}Dark", dark) }
    private val monoVector by lazy(LazyThreadSafetyMode.NONE) { build("${name}Mono", mono) }

    fun vector(look: WontonGlyphLook): ImageVector = when (look) {
        WontonGlyphLook.Light -> lightVector
        WontonGlyphLook.Dark -> darkVector
        WontonGlyphLook.Mono -> monoVector
    }

    fun labelColor(look: WontonGlyphLook): Color? = label?.let {
        val table = when (look) {
            WontonGlyphLook.Light -> light
            WontonGlyphLook.Dark -> dark
            WontonGlyphLook.Mono -> mono
        }
        Color(table[it.color])
    }
}

enum class WontonGlyphLook { Light, Dark, Mono }

/**
 * 单色版的图是灰度图：黑是实的，越浅越淡，白是纸面和字。这个矩阵把颜色统一成 [tint]，
 * 不透明度取“（原来的不透明度 − 亮度）× tint 的不透明度”，白的地方就透了，等于把下面挖空。
 */
private fun monoFilter(tint: Color): ColorFilter {
    val full = 255f
    return ColorFilter.colorMatrix(
        ColorMatrix(
            floatArrayOf(
                0f, 0f, 0f, 0f, tint.red * full,
                0f, 0f, 0f, 0f, tint.green * full,
                0f, 0f, 0f, 0f, tint.blue * full,
                -LUMA_RED * tint.alpha, -LUMA_GREEN * tint.alpha, -LUMA_BLUE * tint.alpha, tint.alpha, 0f,
            ),
        ),
    )
}

private const val LUMA_RED = 0.2126f
private const val LUMA_GREEN = 0.7152f
private const val LUMA_BLUE = 0.0722f

/**
 * 画一个图标，参数和 Material 的 `Icon` 一样。彩色时 [tint] 不起作用（图标有自己的颜色），单色时图标取 [tint]。
 */
@Composable
fun Icon(
    icon: WontonGlyph,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current,
) {
    val look = when {
        LocalWontonIconMono.current -> WontonGlyphLook.Mono
        WontonTheme.isDark -> WontonGlyphLook.Dark
        else -> WontonGlyphLook.Light
    }
    val filter = if (look == WontonGlyphLook.Mono) remember(tint) { monoFilter(tint) } else null
    Image(
        rememberVectorPainter(icon.vector(look)),
        contentDescription,
        modifier.glyphLabel(icon, look, tint),
        colorFilter = filter,
    )
}

@Composable
private fun Modifier.glyphLabel(icon: WontonGlyph, look: WontonGlyphLook, tint: Color): Modifier {
    val label = icon.label ?: return this
    val measurer = rememberTextMeasurer()
    val mono = look == WontonGlyphLook.Mono
    val color = if (mono) tint else icon.labelColor(look) ?: tint
    // 单色时字要把底挖空：先画进离屏层，字用 DstOut 擦掉底下的像素。
    return graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }.drawWithContent {
        drawContent()
        val side = size.minDimension
        val layout = measurer.measure(
            label.text,
            TextStyle(fontSize = (side * label.size).toSp(), fontWeight = if (label.bold) FontWeight.Bold else null),
        )
        val left = side * label.centerX - layout.size.width / 2f
        drawText(
            layout,
            color = color,
            topLeft = Offset(left, side * label.baseline - layout.firstBaseline),
            blendMode = if (mono) BlendMode.DstOut else BlendMode.SrcOver,
        )
    }
}
