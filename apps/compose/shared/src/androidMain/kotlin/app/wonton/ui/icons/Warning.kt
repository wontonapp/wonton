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

private const val P0 = "M14.112 3.832Q15.5 1.4 16.888 3.832L26.41 20.516Q27.6 22.6 25.2 22.6L5.8 22.6Q3.4 22.6 4.59 20.516Z"
private const val P1 = "M15.5 9.2V15"
private const val P2 = "M13.9 19A1.6 1.6 0 1 0 17.1 19A1.6 1.6 0 1 0 13.9 19Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.616f, endY = 22.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.616f, endY = 22.6f))
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    clearGroup()
}.build()

val WontonIcon.Warning: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Warning",
        light = longArrayOf(0xFFC09D64, 0xFFB48B47, 0xFFE1D1B6, 0xFFCFB489, 0xFF9B6308, 0xFFFFFFFF),
        dark = longArrayOf(0xFFCE9F52, 0xFFCF963B, 0xFFCDB792, 0xFFCEAA6F, 0xFFD0840A, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
