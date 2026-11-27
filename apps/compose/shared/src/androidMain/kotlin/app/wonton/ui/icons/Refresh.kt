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
private const val P1 = "M7.5 12.5A8 8 0 1 0 23.5 12.5A8 8 0 1 0 7.5 12.5Z"
private const val P2 = "M-3.604 -2.524A4.4 4.4 0 0 1 2.2 -3.811M3.604 2.524A4.4 4.4 0 0 1 -2.2 3.811"
private const val P3 = "M3.2 -5.543Q3.45 -5.976 3.623 -5.506L4.625 -2.78Q4.798 -2.311 4.305 -2.225L1.443 -1.731Q0.95 -1.645 1.2 -2.078ZM-3.2 5.543Q-3.45 5.976 -3.623 5.506L-4.625 2.78Q-4.798 2.311 -4.305 2.225L-1.443 1.731Q-0.95 1.645 -1.2 2.078Z"

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
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 4.5f, endY = 20.5f))
    addGroup(translationX = 15.5f, translationY = 12.5f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Refresh: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Refresh",
        light = longArrayOf(0xFF69A9CB, 0xFF4D99C2, 0xFFB8D6E7, 0xFF8DBDD8, 0xFF1076AD, 0xFF005189),
        dark = longArrayOf(0xFF5AAEDC, 0xFF44A9DF, 0xFF96BED4, 0xFF75B5D8, 0xFF169DE6, 0xFF006DB7),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
