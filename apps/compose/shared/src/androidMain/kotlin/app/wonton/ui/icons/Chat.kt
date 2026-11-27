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

private const val P0 = "M4 5.5Q4 3.3 6.2 3.3L17.8 3.3Q20 3.3 20 5.5L20 12.7Q20 14.9 17.8 14.9L6.2 14.9Q4 14.9 4 12.7Z"
private const val P1 = "M7 6.8H13.4"
private const val P2 = "M10.2 11Q10.2 8.8 12.4 8.8L24.8 8.8Q27 8.8 27 11L27 16.8Q27 19 24.8 19L17.9 19Q17.4 19 17.046 19.354L14.224 22.176Q13.8 22.6 13.8 22L13.8 19.5Q13.8 19 13.3 19L12 19Q10.2 19 10.2 16.8Z"
private const val P3 = "M13.2 13.9A1.4 1.4 0 1 0 16 13.9A1.4 1.4 0 1 0 13.2 13.9Z"
private const val P4 = "M17.2 13.9A1.4 1.4 0 1 0 20 13.9A1.4 1.4 0 1 0 17.2 13.9Z"
private const val P5 = "M21.2 13.9A1.4 1.4 0 1 0 24 13.9A1.4 1.4 0 1 0 21.2 13.9Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.3f, endY = 14.9f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.3f, endY = 14.9f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 8.8f, endY = 22.351f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 8.8f, endY = 22.351f))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[6])))
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[6])))
    addPath(pathData = addPathNodes(P5), fill = SolidColor(Color(c[6])))
    clearGroup()
}.build()

val WontonIcon.Chat: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Chat",
        light = longArrayOf(0xFF679FF7, 0xFF4B8DF6, 0xFFB7D2FB, 0xFF8BB6F9, 0xFF0D66F3, 0xFFFFFFFF, 0xFF2439C5),
        dark = longArrayOf(0xFF82A9E7, 0xFF75A2EC, 0xFFA9BBD9, 0xFF94B1E0, 0xFF5794F7, 0xFFFFFFFF, 0xFF4364C3),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF, 0xFF000000),
        build = ::art,
    )
}
