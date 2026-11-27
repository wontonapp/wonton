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

private const val P0 = "M4 3.2Q4 1 6.2 1L14 1Q16.2 1 16.2 3.2L16.2 21.8Q16.2 24 14 24L6.2 24Q4 24 4 21.8Z"
private const val P1 = "M6.8 5.6Q6.8 4 8.4 4L11.8 4Q13.4 4 13.4 5.6L13.4 24Q13.4 24 13.4 24L6.8 24Q6.8 24 6.8 24Z"
private const val P2 = "M8.9 14A1.2 1.2 0 1 0 11.3 14A1.2 1.2 0 1 0 8.9 14Z"
private const val P3 = "M20.3 12.5H24"
private const val P4 = "M23.6 10.4Q23.6 9.9 23.997 10.204L26.603 12.196Q27 12.5 26.603 12.804L23.997 14.796Q23.6 15.1 23.6 14.6Z"

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
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 4f, endY = 24f))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[6])))
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[6])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[6])))
    clearGroup()
}.build()

val WontonIcon.Leave: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Leave",
        light = longArrayOf(0xFFE87276, 0xFFE3585C, 0xFFF4BDBE, 0xFFED9497, 0xFFDA1F25, 0xFFFFFFFF, 0xFFAA0000),
        dark = longArrayOf(0xFFDF9193, 0xFFE28689, 0xFFD5B0B1, 0xFFDA9FA1, 0xFFEA6E72, 0xFFFFFFFF, 0xFFB4453C),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF, 0xFF000000),
        build = ::art,
    )
}
