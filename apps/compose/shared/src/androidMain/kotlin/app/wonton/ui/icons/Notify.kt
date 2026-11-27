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

private const val P0 = "M4.468 8.815A11.6 11.6 0 0 1 9.013 2.783M21.987 2.783A11.6 11.6 0 0 1 26.532 8.815"
private const val P1 = "M13.7 5.02A1.8 1.8 0 1 0 17.3 5.02A1.8 1.8 0 1 0 13.7 5.02Z"
private const val P2 = "M13.3 21.08A2.2 2.2 0 1 0 17.7 21.08A2.2 2.2 0 1 0 13.3 21.08Z"
private const val P3 = "M8.79 18C9.89 16.68 10.22 14.7 10.22 12.06C10.22 8.32 12.53 5.9 15.5 5.9C18.47 5.9 20.78 8.32 20.78 12.06C20.78 14.7 21.11 16.68 22.21 18Z"
private const val P4 = "M7.03 18.99Q7.03 17.12 8.9 17.12L22.1 17.12Q23.97 17.12 23.97 18.99L23.97 18.99Q23.97 20.86 22.1 20.86L8.9 20.86Q7.03 20.86 7.03 18.99Z"

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
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[4])))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[4])))
    addPath(pathData = addPathNodes(P3), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 5.9f, endY = 18f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 5.9f, endY = 18f))
    addPath(pathData = addPathNodes(P4), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 17.12f, endY = 20.86f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P4), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 17.12f, endY = 20.86f))
    clearGroup()
}.build()

val WontonIcon.Notify: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Notify",
        light = longArrayOf(0xFFC09D64, 0xFFB48B47, 0xFFE1D1B6, 0xFFCFB489, 0xFF714600, 0xFF9B6308),
        dark = longArrayOf(0xFFCE9F52, 0xFFCF963B, 0xFFCDB792, 0xFFCEAA6F, 0xFF995E00, 0xFFD0840A),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
