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

private const val P0 = "M4 12.5Q4 6.5 10 6.5L21 6.5Q27 6.5 27 12.5L27 12.5Q27 18.5 21 18.5L10 18.5Q4 18.5 4 12.5Z"
private const val P1 = "M7.8 12.5A2 2 0 1 0 11.8 12.5A2 2 0 1 0 7.8 12.5Z"
private const val P2 = "M13.5 12.5A2 2 0 1 0 17.5 12.5A2 2 0 1 0 13.5 12.5Z"
private const val P3 = "M19.2 12.5A2 2 0 1 0 23.2 12.5A2 2 0 1 0 19.2 12.5Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 6.5f, endY = 18.5f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 6.5f, endY = 18.5f))
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    clearGroup()
}.build()

val WontonIcon.More: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "More",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFFFFFFFF),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
