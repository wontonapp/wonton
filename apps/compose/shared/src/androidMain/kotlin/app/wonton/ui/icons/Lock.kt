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

private const val P0 = "M10.5 12V7.6A5 5 0 0 1 20.5 7.6V12"
private const val P1 = "M5.6 13.2Q5.6 11 7.8 11L23.2 11Q25.4 11 25.4 13.2L25.4 21.4Q25.4 23.6 23.2 23.6L7.8 23.6Q5.6 23.6 5.6 21.4Z"
private const val P2 = "M13 16.2A2.5 2.5 0 1 0 18 16.2A2.5 2.5 0 1 0 13 16.2Z"
private const val P3 = "M14.6 17.5Q14.6 16.6 15.5 16.6L15.5 16.6Q16.4 16.6 16.4 17.5L16.4 20.3Q16.4 21.2 15.5 21.2L15.5 21.2Q14.6 21.2 14.6 20.3Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 3.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[5])), strokeAlpha = 0.45f, strokeLineWidth = 1f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 11f, endY = 23.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[6])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 11f, endY = 23.6f))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[7])))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[7])))
    clearGroup()
}.build()

val WontonIcon.Lock: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Lock",
        light = longArrayOf(0xFF6CAF8B, 0xFF51A075, 0xFFBAD9C8, 0xFF8FC2A7, 0xFF54A278, 0xFFFFFFFF, 0xFF158046, 0xFF005F34),
        dark = longArrayOf(0xFF5DB787, 0xFF49B37A, 0xFF98C2AB, 0xFF78BC97, 0xFF4BB47B, 0xFFFFFFFF, 0xFF1CAB5E, 0xFF008047),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF444444, 0xFFFFFFFF, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
