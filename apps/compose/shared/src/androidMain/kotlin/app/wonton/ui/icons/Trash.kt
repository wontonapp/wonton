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

private const val P0 = "M7.441 6.599Q7.4 6 8 6L23 6Q23.6 6 23.559 6.599L22.593 20.607Q22.4 23.4 19.6 23.4L11.4 23.4Q8.6 23.4 8.407 20.607Z"
private const val P1 = "M0 1.2Q0 0 1.2 0L9.2 0Q10.4 0 10.4 1.2L10.4 6.4Q10.4 7.6 9.2 7.6L1.2 7.6Q0 7.6 0 6.4Z"
private const val P2 = "M1.6 3A1.4 1.4 0 1 0 4.4 3A1.4 1.4 0 1 0 1.6 3Z"
private const val P3 = "M6.91 7.195Q6.8 6 8 6L23 6Q24.2 6 24.09 7.195L22.856 20.612Q22.6 23.4 19.8 23.4L11.2 23.4Q8.4 23.4 8.144 20.612Z"
private const val P4 = "M13.064 9.4L13.512 20.2M17.936 9.4L17.488 20.2"
private const val P5 = "M6.4 6.6Q5.4 6.6 5.4 5.6L5.4 5Q5.4 4 6.4 4L11.9 4Q12.4 4 12.4 3.5L12.4 2.4Q12.4 1.6 13.2 1.6L17.8 1.6Q18.6 1.6 18.6 2.4L18.6 3.5Q18.6 4 19.1 4L24.6 4Q25.6 4 25.6 5L25.6 5.6Q25.6 6.6 24.6 6.6Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 0.935f to Color(c[4]), 0.935f to Color(c[5]), 1f to Color(c[6]), startY = 6f, endY = 23.4f))
    addGroup(translationX = 10.3f, translationY = 12f)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 0f, endY = 7.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[9])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 0f, endY = 7.6f))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[10])))
    clearGroup()
    addPath(pathData = addPathNodes(P3), stroke = Brush.verticalGradient(0f to Color(c[2]), 0.931f to Color(c[7]), 0.931f to Color(c[8]), 1f to Color(c[6]), startY = 6f, endY = 23.4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[9])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[2]), 0.931f to Color(c[7]), 0.931f to Color(c[8]), 1f to Color(c[6]), startY = 6f, endY = 23.4f))
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[9])), strokeAlpha = 0.6f, strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P5), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.6f, endY = 6.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P5), stroke = SolidColor(Color(c[9])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P5), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.6f, endY = 6.6f))
    clearGroup()
}.build()

val WontonIcon.Trash: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Trash",
        light = longArrayOf(0xFFE87276, 0xFFE3585C, 0xFFF4BDBE, 0xFFED9497, 0xFFE45B5F, 0xFFC51C22, 0xFFE9787B, 0xFFEE9A9D, 0xFFE66569, 0xFFDA1F25, 0xFFFFFFFF),
        dark = longArrayOf(0xFFDF9193, 0xFFE28689, 0xFFD5B0B1, 0xFFDA9FA1, 0xFFE2878A, 0xFFD46467, 0xFFDE9396, 0xFFD9A2A4, 0xFFE18C8E, 0xFFEA6E72, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF444444, 0xFF000000, 0xFF656565, 0xFF8D8D8D, 0xFF505050, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
