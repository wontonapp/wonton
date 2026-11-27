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

private const val P0 = "M9.4 12.5L21.8 6.4M9.4 12.5L21.8 18.6"
private const val P1 = "M5 12.5A4.4 4.4 0 1 0 13.8 12.5A4.4 4.4 0 1 0 5 12.5Z"
private const val P2 = "M-4 0A4 4 0 1 0 4 0A4 4 0 1 0 -4 0Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 8.1f, endY = 16.9f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 8.1f, endY = 16.9f))
    addGroup(translationX = 21.8f, translationY = 6.4f)
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -4f, endY = 4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -4f, endY = 4f))
    clearGroup()
    addGroup(translationX = 21.8f, translationY = 18.6f)
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -4f, endY = 4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -4f, endY = 4f))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Share: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Share",
        light = longArrayOf(0xFF679FF7, 0xFF4B8DF6, 0xFFB7D2FB, 0xFF8BB6F9, 0xFF2439C5, 0xFF0D66F3),
        dark = longArrayOf(0xFF82A9E7, 0xFF75A2EC, 0xFFA9BBD9, 0xFF94B1E0, 0xFF4364C3, 0xFF5794F7),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
