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

private const val P0 = "M15.5 16.4V23.6"
private const val P1 = "M12.2 4.4Q12.2 4.4 12.2 4.4L18.8 4.4Q18.8 4.4 18.8 4.4L18.8 8.4Q18.8 9.6 19.506 10.57L22 14Q22 14 22 14L9 14Q9 14 9 14L11.494 10.57Q12.2 9.6 12.2 8.4Z"
private const val P2 = "M9.8 3.6Q9.8 1.4 12 1.4L19 1.4Q21.2 1.4 21.2 3.6L21.2 3.6Q21.2 5.8 19 5.8L12 5.8Q9.8 5.8 9.8 3.6Z"
private const val P3 = "M7.2 15.2Q7.2 13 9.4 13L21.6 13Q23.8 13 23.8 15.2L23.8 15.2Q23.8 17.4 21.6 17.4L9.4 17.4Q7.2 17.4 7.2 15.2Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 4.4f, endY = 14f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 4.4f, endY = 14f))
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.4f, endY = 5.8f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.4f, endY = 5.8f))
    addPath(pathData = addPathNodes(P3), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 13f, endY = 17.4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 13f, endY = 17.4f))
    clearGroup()
}.build()

val WontonIcon.Pin: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Pin",
        light = longArrayOf(0xFFE87276, 0xFFE3585C, 0xFFF4BDBE, 0xFFED9497, 0xFFAA0000, 0xFFDA1F25),
        dark = longArrayOf(0xFFDF9193, 0xFFE28689, 0xFFD5B0B1, 0xFFDA9FA1, 0xFFB4453C, 0xFFEA6E72),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
