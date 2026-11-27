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

private const val P0 = "M9 21.6A5.7 5.7 0 0 1 8.304 10.243A7.5 7.5 0 1 1 23.191 11.277A5.2 5.2 0 0 1 22.3 21.6Z"
private const val P1 = "M15.5 14V6.4M12.1 9.8L15.5 6.4L18.9 9.8"
private const val P2 = "M5.785 18.2Q3.785 18.2 4.893 19.852A5.7 5.7 0 0 0 9 21.6L22.3 21.6A5.2 5.2 0 0 0 26.147 19.899Q27.179 18.2 25.179 18.2Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.5202f, endY = 21.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.5202f, endY = 21.6f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 18.2f, endY = 21.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 18.2f, endY = 21.6f))
    clearGroup()
}.build()

val WontonIcon.Upload: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Upload",
        light = longArrayOf(0xFF69A9CB, 0xFF4D99C2, 0xFFB8D6E7, 0xFF8DBDD8, 0xFF1076AD, 0xFFFFFFFF),
        dark = longArrayOf(0xFF5AAEDC, 0xFF44A9DF, 0xFF96BED4, 0xFF75B5D8, 0xFF169DE6, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
