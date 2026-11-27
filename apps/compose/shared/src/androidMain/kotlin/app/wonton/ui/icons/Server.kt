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

private const val P0 = "M5.2 3.1Q5.2 1.3 7 1.3L24 1.3Q25.8 1.3 25.8 3.1L25.8 6.1Q25.8 7.9 24 7.9L7 7.9Q5.2 7.9 5.2 6.1Z"
private const val P1 = "M7.7 4.6A1.3 1.3 0 1 0 10.3 4.6A1.3 1.3 0 1 0 7.7 4.6Z"
private const val P2 = "M12.4 4.6H16"
private const val P3 = "M5.2 10.7Q5.2 8.9 7 8.9L24 8.9Q25.8 8.9 25.8 10.7L25.8 13.7Q25.8 15.5 24 15.5L7 15.5Q5.2 15.5 5.2 13.7Z"
private const val P4 = "M7.7 12.2A1.3 1.3 0 1 0 10.3 12.2A1.3 1.3 0 1 0 7.7 12.2Z"
private const val P5 = "M12.4 12.2H16"
private const val P6 = "M5.2 18.3Q5.2 16.5 7 16.5L24 16.5Q25.8 16.5 25.8 18.3L25.8 21.3Q25.8 23.1 24 23.1L7 23.1Q5.2 23.1 5.2 21.3Z"
private const val P7 = "M7.7 19.8A1.3 1.3 0 1 0 10.3 19.8A1.3 1.3 0 1 0 7.7 19.8Z"
private const val P8 = "M12.4 19.8H16"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.3f, endY = 7.9f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.3f, endY = 7.9f))
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P3), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 8.9f, endY = 15.5f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 8.9f, endY = 15.5f))
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P5), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P6), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 16.5f, endY = 23.1f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P6), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P6), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 16.5f, endY = 23.1f))
    addPath(pathData = addPathNodes(P7), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P8), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    clearGroup()
}.build()

val WontonIcon.Server: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Server",
        light = longArrayOf(0xFF66ADA7, 0xFF499E97, 0xFFB7D8D5, 0xFF8AC1BC, 0xFF0B7D73, 0xFFFFFFFF),
        dark = longArrayOf(0xFF55B5AC, 0xFF3FB0A6, 0xFF94C1BD, 0xFF72BAB4, 0xFF0FA799, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
