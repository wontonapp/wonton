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

private const val P0 = "M4 12.5A11.5 11.5 0 1 0 27 12.5A11.5 11.5 0 1 0 4 12.5Z"
private const val P1 = "M3 12.5H28"
private const val P2 = "M10.9 12.5A4.6 11.5 0 1 0 20.1 12.5A4.6 11.5 0 1 0 10.9 12.5Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1f, endY = 24f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1f, endY = 24f))
    addGroup(clipPathData = addPathNodes(P0))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Globe: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Globe",
        light = longArrayOf(0xFF66ADA7, 0xFF499E97, 0xFFB7D8D5, 0xFF8AC1BC, 0xFF0B7D73, 0xFFFFFFFF),
        dark = longArrayOf(0xFF55B5AC, 0xFF3FB0A6, 0xFF94C1BD, 0xFF72BAB4, 0xFF0FA799, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
