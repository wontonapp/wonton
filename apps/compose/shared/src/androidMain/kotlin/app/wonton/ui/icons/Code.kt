// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M4 5.5Q4 3.3 6.2 3.3L24.8 3.3Q27 3.3 27 5.5L27 19.5Q27 21.7 24.8 21.7L6.2 21.7Q4 21.7 4 19.5Z"
private const val P1 = "M6 11.1Q6 9.9 7.2 9.9L23.8 9.9Q25 9.9 25 11.1L25 18.5Q25 19.7 23.8 19.7L7.2 19.7Q6 19.7 6 18.5Z"
private const val P2 = "M12.2 12.2L9.5 14.8L12.2 17.4M18.8 12.2L21.5 14.8L18.8 17.4"
private const val P3 = "M16.6 12.1L14.4 17.5"
private const val P4 = "M4 5.5Q4 3.3 6.2 3.3L24.8 3.3Q27 3.3 27 5.5L27 7.9Q27 7.9 27 7.9L4 7.9Q4 7.9 4 7.9Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.3f, endY = 21.7f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.3f, endY = 21.7f))
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[6])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[7])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P4), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 3.3f, endY = 7.9f))
    clearGroup()
}.build()

val WontonIcon.Code: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Code",
        light = longArrayOf(0xFF8F9CF0, 0xFF7A8AED, 0xFFCAD0F8, 0xFFAAB4F3, 0xFF4D62E7, 0xFFFFFFFF, 0xFF4338B7, 0xFF8593EF),
        dark = longArrayOf(0xFF9CA5E1, 0xFF939EE5, 0xFFB5BAD6, 0xFFA7AEDC, 0xFF7F8EEE, 0xFFFFFFFF, 0xFF635FB8, 0xFF97A1E3),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF, 0xFF000000, 0xFF7A7A7A),
        build = ::art,
    )
}
