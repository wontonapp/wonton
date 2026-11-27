// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M4 6Q4 3.8 6.2 3.8L24.8 3.8Q27 3.8 27 6L27 19Q27 21.2 24.8 21.2L6.2 21.2Q4 21.2 4 19Z"
private const val P1 = "M4 14.4Q4 12.6 5.8 12.6L25.2 12.6Q27 12.6 27 14.4L27 19Q27 21.2 24.8 21.2L6.2 21.2Q4 21.2 4 19Z"
private const val P2 = "M12 16.9H16.4"
private const val P3 = "M7 16.9A1.4 1.4 0 1 0 9.8 16.9A1.4 1.4 0 1 0 7 16.9Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.8f, endY = 21.2f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.8f, endY = 21.2f))
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 12.6f, endY = 21.2f))
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[6])))
    clearGroup()
}.build()

val WontonIcon.Disk: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Disk",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFF3E475B, 0xFF8C96A4),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFF616B7E, 0xFF9DA7B5),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFF000000, 0xFF7A7A7A),
        build = ::art,
    )
}
