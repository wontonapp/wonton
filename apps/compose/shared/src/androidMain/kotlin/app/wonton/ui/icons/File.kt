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

private const val P0 = "M6.3 3.7Q6.3 1.5 8.5 1.5L18.2 1.5Q18.7 1.5 19.054 1.854L24.346 7.146Q24.7 7.5 24.7 8L24.7 21.3Q24.7 23.5 22.5 23.5L8.5 23.5Q6.3 23.5 6.3 21.3Z"
private const val P1 = "M9.6 13.4H21.4M9.6 16.4H17.8"
private const val P2 = "M18.7 1.9Q18.7 1.5 18.983 1.783L24.417 7.217Q24.7 7.5 24.3 7.5L19.9 7.5Q18.7 7.5 18.7 6.3Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.5f, endY = 23.5f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.5f, endY = 23.5f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.6661f, endY = 7.5f))
    clearGroup()
}.build()

val WontonIcon.File: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "File",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFFFFFFFF),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
