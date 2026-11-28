// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M2.6 12.5Q2.6 10.1 5 10.1L15.2 10.1Q17.6 10.1 17.6 12.5L17.6 12.5Q17.6 14.9 15.2 14.9L5 14.9Q2.6 14.9 2.6 12.5Z"
private const val P1 = "M27.099 10.1H22.4Q21.8 10.1 21.8 10.7V14.3Q21.8 14.9 22.4 14.9H27.099A6 6 0 1 1 27.099 10.1Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(rotate = -45f, pivotX = 15.5f, pivotY = 12.5f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 10.1f, endY = 14.9f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 10.1f, endY = 14.9f))
    clearGroup()
    addGroup(rotate = -45f, pivotX = 15.5f, pivotY = 12.5f)
    clearGroup()
    addGroup(rotate = -45f, pivotX = 15.5f, pivotY = 12.5f)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 6.555f, endY = 18.445f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 6.555f, endY = 18.445f))
    clearGroup()
    addGroup(rotate = -45f, pivotX = 15.5f, pivotY = 12.5f)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Wrench: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Wrench",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000),
        build = ::art,
    )
}
