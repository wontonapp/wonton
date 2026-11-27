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

private const val P0 = "M7.4 12.8Q7.4 10.6 9.6 10.6L21.4 10.6Q23.6 10.6 23.6 12.8L23.6 20.8Q23.6 23 21.4 23L9.6 23Q7.4 23 7.4 20.8Z"
private const val P1 = "M13.2 16.8Q13.2 15.6 14.4 15.6L16.6 15.6Q17.8 15.6 17.8 16.8L17.8 23Q17.8 23 17.8 23L13.2 23Q13.2 23 13.2 23Z"
private const val P2 = "M5 12.6Q3.6 12.6 4.663 11.689L13.83 3.832Q15.5 2.4 17.17 3.832L26.337 11.689Q27.4 12.6 26 12.6Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 10.6f, endY = 23f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 10.6f, endY = 23f))
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[4]), 1f to Color(c[4]), startY = 15.6f, endY = 23f))
    addGroup(translationX = 13.2f, translationY = 0f)
    addGroup(translationX = -13.2f, translationY = 0f)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[6])))
    clearGroup()
    clearGroup()
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 3.116f, endY = 12.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 3.116f, endY = 12.6f))
    clearGroup()
}.build()

val WontonIcon.Home: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Home",
        light = longArrayOf(0xFFD0926C, 0xFFC87E51, 0xFFE9CCBA, 0xFFDCAC8F, 0xFF893400, 0xFFB55215, 0xFFFFFFFF),
        dark = longArrayOf(0xFFDD9569, 0xFFE08B57, 0xFFD4B29D, 0xFFD9A281, 0xFFAE4E00, 0xFFE7752F, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
