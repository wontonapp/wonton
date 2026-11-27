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

private const val P0 = "M0 2.2Q0 0 2.2 0L11.4 0Q13.6 0 13.6 2.2L13.6 13.4Q13.6 15.6 11.4 15.6L2.2 15.6Q0 15.6 0 13.4Z"
private const val P1 = "M2.6 4.2H11M2.6 7.2H8.2"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 5.2f, translationY = 2.2f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 0f, endY = 15.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[7])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 0f, endY = 15.6f))
    clearGroup()
    addGroup(translationX = 12.2f, translationY = 7.2f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[2]), 0.923f to Color(c[4]), 0.923f to Color(c[5]), 1f to Color(c[6]), startY = 0f, endY = 15.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[7])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[2]), 0.923f to Color(c[4]), 0.923f to Color(c[5]), 1f to Color(c[6]), startY = 0f, endY = 15.6f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[8])), strokeAlpha = 0.55f, strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Copy: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Copy",
        light = longArrayOf(0xFF69A9CB, 0xFF4D99C2, 0xFFB8D6E7, 0xFF8DBDD8, 0xFF94C1DA, 0xFF5BA1C7, 0xFF6FACCE, 0xFF1076AD, 0xFF005189),
        dark = longArrayOf(0xFF5AAEDC, 0xFF44A9DF, 0xFF96BED4, 0xFF75B5D8, 0xFF7AB7D8, 0xFF4FACDE, 0xFF5EB0DC, 0xFF169DE6, 0xFF006DB7),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF919191, 0xFF7A7A7A, 0xFF828282, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
