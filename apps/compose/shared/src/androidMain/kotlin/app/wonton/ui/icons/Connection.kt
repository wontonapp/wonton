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

private const val P0 = "M15.182 9.548Q15.5 8.6 15.818 9.548L20.218 22.662Q20.6 23.8 19.4 23.8L11.6 23.8Q10.4 23.8 10.782 22.662Z"
private const val P1 = "M13.6 19.2H17.4"
private const val P2 = "M12.5 7.4A3 3 0 1 0 18.5 7.4A3 3 0 1 0 12.5 7.4Z"
private const val P3 = "M20.228 3.706A6 6 0 0 1 20.228 11.094M10.772 11.094A6 6 0 0 1 10.772 3.706"
private const val P4 = "M23.065 1.49A9.6 9.6 0 0 1 23.065 13.31M7.935 13.31A9.6 9.6 0 0 1 7.935 1.49"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 9.074f, endY = 23.8f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 9.074f, endY = 23.8f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 4.4f, endY = 10.4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 4.4f, endY = 10.4f))
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[6])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[6])), strokeAlpha = 0.35f, strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    clearGroup()
}.build()

val WontonIcon.Connection: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Connection",
        light = longArrayOf(0xFF66ADA7, 0xFF499E97, 0xFFB7D8D5, 0xFF8AC1BC, 0xFF0B7D73, 0xFFFFFFFF, 0xFF005B59),
        dark = longArrayOf(0xFF55B5AC, 0xFF3FB0A6, 0xFF94C1BD, 0xFF72BAB4, 0xFF0FA799, 0xFFFFFFFF, 0xFF007B77),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF, 0xFF000000),
        build = ::art,
    )
}
