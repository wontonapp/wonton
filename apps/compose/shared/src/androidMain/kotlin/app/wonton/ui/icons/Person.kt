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

private const val P0 = "M4 12.5A11.5 11.5 0 1 0 27 12.5A11.5 11.5 0 1 0 4 12.5Z"
private const val P1 = "M5 27Q5 27 5 27L5 22Q5 22 5 22L7.7 19.7Q10.4 17.4 14.4 17.4L16.6 17.4Q20.6 17.4 23.3 19.7L26 22Q26 22 26 22L26 27Q26 27 26 27Z"
private const val P2 = "M11.1 10.4A4.4 4.4 0 1 0 19.9 10.4A4.4 4.4 0 1 0 11.1 10.4Z"
private const val P3 = "M-5 0A5 5 0 1 0 5 0A5 5 0 1 0 -5 0Z"
private const val P4 = "M-2.5 0H2.5M0 -2.5V2.5"

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
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    clearGroup()
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    addGroup(translationX = 22.6f, translationY = 18.4f)
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[6])))
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Person: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Person",
        light = longArrayOf(0xFFE07BA3, 0xFFDA6292, 0xFFF0C1D4, 0xFFE79BB9, 0xFFCD2D6D, 0xFFFFFFFF, 0xFF33363D),
        dark = longArrayOf(0xFFD993AE, 0xFFDB88A9, 0xFFD2B1BE, 0xFFD5A0B5, 0xFFE0719D, 0xFFFFFFFF, 0xFF33363D),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF, 0xFF33363D),
        build = ::art,
    )
}
