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

private const val P0 = "M12.5 4.4V2.1M12.5 20.6V22.9M7.4 9.5H5.1M23.6 9.5H25.9M18.5 4.4V2.1M18.5 20.6V22.9M7.4 15.5H5.1M23.6 15.5H25.9"
private const val P1 = "M7.4 6.6Q7.4 4.4 9.6 4.4L21.4 4.4Q23.6 4.4 23.6 6.6L23.6 18.4Q23.6 20.6 21.4 20.6L9.6 20.6Q7.4 20.6 7.4 18.4Z"
private const val P2 = "M11.2 10Q11.2 8.2 13 8.2L18 8.2Q19.8 8.2 19.8 10L19.8 15Q19.8 16.8 18 16.8L13 16.8Q11.2 16.8 11.2 15Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 4.4f, endY = 20.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[6])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 4.4f, endY = 20.6f))
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 8.2f, endY = 16.8f))
    clearGroup()
}.build()

val WontonIcon.Cpu: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Cpu",
        light = longArrayOf(0xFF8F9CF0, 0xFF7A8AED, 0xFFCAD0F8, 0xFFAAB4F3, 0xFF4659D1, 0xFF4338B7, 0xFF4D62E7),
        dark = longArrayOf(0xFF9CA5E1, 0xFF939EE5, 0xFFB5BAD6, 0xFFA7AEDC, 0xFF7381D7, 0xFF635FB8, 0xFF7F8EEE),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
