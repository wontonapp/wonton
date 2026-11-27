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

private const val P0 = "M11.1 5.2Q11.1 4.4 11.884 4.241L23.116 1.959Q23.9 1.8 23.9 2.6L23.9 17.3Q23.9 17.9 23.3 17.9L22.3 17.9Q21.7 17.9 21.7 17.3L21.7 5.9Q21.7 5.6 21.408 5.669L13.592 7.531Q13.3 7.6 13.3 7.9L13.3 19.5Q13.3 20.1 12.7 20.1L11.7 20.1Q11.1 20.1 11.1 19.5ZM7.476 21.212A3.6 2.7 -18 1 1 14.324 18.988A3.6 2.7 -18 1 1 7.476 21.212ZM18.076 19.012A3.6 2.7 -18 1 1 24.924 16.788A3.6 2.7 -18 1 1 18.076 19.012Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.9331f, endY = 22.8981f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.9331f, endY = 22.8981f))
    clearGroup()
}.build()

val WontonIcon.Music: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Music",
        light = longArrayOf(0xFFE07BA3, 0xFFDA6292, 0xFFF0C1D4, 0xFFE79BB9, 0xFFCD2D6D),
        dark = longArrayOf(0xFFD993AE, 0xFFDB88A9, 0xFFD2B1BE, 0xFFD5A0B5, 0xFFE0719D),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000),
        build = ::art,
    )
}
