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
private const val P1 = "M7.3 12.5A8.2 8.2 0 1 0 23.7 12.5A8.2 8.2 0 1 0 7.3 12.5Z"
private const val P2 = "M15.5 12.5L20.702 7.816A7 7 0 0 1 20.702 7.816Z"
private const val P3 = "M15.5 12.5V8.3"
private const val P4 = "M15.5 12.5V6.2"
private const val P5 = "M13.8 12.5A1.7 1.7 0 1 0 17.2 12.5A1.7 1.7 0 1 0 13.8 12.5Z"

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
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 7.816f, endY = 12.5f))
    addGroup(rotate = 304f, pivotX = 15.5f, pivotY = 12.5f)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[6])), strokeLineWidth = 2.6f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    addGroup(rotate = 3648f, pivotX = 15.5f, pivotY = 12.5f)
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[7])), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    addPath(pathData = addPathNodes(P5), fill = SolidColor(Color(c[7])))
    clearGroup()
}.build()

val WontonIcon.Clock: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Clock",
        light = longArrayOf(0xFF69A9CB, 0xFF4D99C2, 0xFFB8D6E7, 0xFF8DBDD8, 0xFF1076AD, 0xFFFFFFFF, 0xFF5BA1C7, 0xFF005189),
        dark = longArrayOf(0xFF5AAEDC, 0xFF44A9DF, 0xFF96BED4, 0xFF75B5D8, 0xFF169DE6, 0xFFFFFFFF, 0xFF4FACDE, 0xFF006DB7),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF, 0xFF7A7A7A, 0xFF000000),
        build = ::art,
    )
}
