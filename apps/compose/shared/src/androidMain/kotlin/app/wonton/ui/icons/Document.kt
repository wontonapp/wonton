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

private const val P0 = "M6.3 3.7Q6.3 1.5 8.5 1.5L18.2 1.5Q18.7 1.5 19.054 1.854L24.346 7.146Q24.7 7.5 24.7 8L24.7 21.3Q24.7 23.5 22.5 23.5L8.5 23.5Q6.3 23.5 6.3 21.3Z"
private const val P1 = "M9.6 5.4H16"
private const val P2 = "M9 10.6Q9 9.4 10.2 9.4L20.8 9.4Q22 9.4 22 10.6L22 19Q22 20.2 20.8 20.2L10.2 20.2Q9 20.2 9 19Z"
private const val P3 = "M11.4 12.6H19.6"
private const val P4 = "M11.4 15.6H16"
private const val P5 = "M18.7 1.9Q18.7 1.5 18.983 1.783L24.417 7.217Q24.7 7.5 24.3 7.5L19.9 7.5Q18.7 7.5 18.7 6.3Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.5f, endY = 23.5f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.5f, endY = 23.5f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[4])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[4])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P5), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.6661f, endY = 7.5f))
    clearGroup()
}.build()

val WontonIcon.Document: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Document",
        light = longArrayOf(0xFFD0926C, 0xFFC87E51, 0xFFE9CCBA, 0xFFDCAC8F, 0xFFB55215, 0xFFFFFFFF),
        dark = longArrayOf(0xFFDD9569, 0xFFE08B57, 0xFFD4B29D, 0xFFD9A281, 0xFFE7752F, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
