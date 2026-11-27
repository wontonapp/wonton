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

private const val P0 = "M4 3.2Q4 1 6.2 1L18.6 1Q20.8 1 20.8 3.2L20.8 19Q20.8 21.2 18.6 21.2L6.2 21.2Q4 21.2 4 19Z"
private const val P1 = "M7.4 4.8H17.4M7.4 7.8H14"
private const val P2 = "M4 12.4Q4 10.8 5.6 10.8L19.2 10.8Q20.8 10.8 20.8 12.4L20.8 19Q20.8 21.2 18.6 21.2L6.2 21.2Q4 21.2 4 19Z"
private const val P3 = "M-3.6 0A3.6 3.6 0 1 0 3.6 0A3.6 3.6 0 1 0 -3.6 0Z"
private const val P4 = "M-4.5 0A4.5 4.5 0 1 0 4.5 0A4.5 4.5 0 1 0 -4.5 0Z"
private const val P5 = "M3.8 3.8L6.6 6.6"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 0.941f to Color(c[4]), 0.941f to Color(c[5]), 1f to Color(c[6]), startY = 1f, endY = 21.2f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[7])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 0.941f to Color(c[4]), 0.941f to Color(c[5]), 1f to Color(c[6]), startY = 1f, endY = 21.2f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[8])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 10.8f, endY = 21.2f))
    addGroup(translationX = 19.8f, translationY = 16.8f)
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[8])), fillAlpha = 0.8f)
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -3.6f, endY = 3.6f), fillAlpha = 0.35f)
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[9])), strokeLineWidth = 1.8f)
    addPath(pathData = addPathNodes(P5), stroke = SolidColor(Color(c[9])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Search: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Search",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF858F9E, 0xFF505C6F, 0xFF9AA3AF, 0xFF58667B, 0xFFFFFFFF, 0xFF3E475B),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF9AA4B3, 0xFF7B889A, 0xFFA3ABB8, 0xFF8896AA, 0xFFFFFFFF, 0xFF616B7E),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF444444, 0xFF000000, 0xFF656565, 0xFF000000, 0xFFFFFFFF, 0xFF000000),
        build = ::art,
    )
}
