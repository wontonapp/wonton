// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M4 8.2Q4 6 6.2 6L17.6 6Q19.8 6 19.8 8.2L19.8 16.8Q19.8 19 17.6 19L6.2 19Q4 19 4 16.8Z"
private const val P1 = "M18.6 10.2Q18.6 9.6 19.182 9.454L24.842 8.04Q26.2 7.7 26.2 9.1L26.2 15.9Q26.2 17.3 24.842 16.96L19.182 15.546Q18.6 15.4 18.6 14.8Z"
private const val P2 = "M7 10.4A1.8 1.8 0 1 0 10.6 10.4A1.8 1.8 0 1 0 7 10.4Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 6f, endY = 19f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 6f, endY = 19f))
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 7.9737f, endY = 17.0263f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 7.9737f, endY = 17.0263f))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    clearGroup()
}.build()

val WontonIcon.VideoCall: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "VideoCall",
        light = longArrayOf(0xFF679FF7, 0xFF4B8DF6, 0xFFB7D2FB, 0xFF8BB6F9, 0xFF0D66F3, 0xFFFFFFFF),
        dark = longArrayOf(0xFF82A9E7, 0xFF75A2EC, 0xFFA9BBD9, 0xFF94B1E0, 0xFF5794F7, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
