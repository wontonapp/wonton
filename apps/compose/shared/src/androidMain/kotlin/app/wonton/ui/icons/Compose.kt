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

private const val P0 = "M5.4 3.2Q5.4 1 7.6 1L21.6 1Q23.8 1 23.8 3.2L23.8 21.8Q23.8 24 21.6 24L7.6 24Q5.4 24 5.4 21.8Z"
private const val P1 = "M8.4 2.6Q8.4 1 10 1L21.6 1Q23.8 1 23.8 3.2L23.8 21.8Q23.8 24 21.6 24L10 24Q8.4 24 8.4 22.4Z"
private const val P2 = "M12.2 6.4H17.4"
private const val P3 = "M2.9 5.5Q2.9 4.3 4.1 4.3L9.1 4.3Q10.3 4.3 10.3 5.5L10.3 5.5Q10.3 6.7 9.1 6.7L4.1 6.7Q2.9 6.7 2.9 5.5ZM2.9 10.2Q2.9 9 4.1 9L9.1 9Q10.3 9 10.3 10.2L10.3 10.2Q10.3 11.4 9.1 11.4L4.1 11.4Q2.9 11.4 2.9 10.2ZM2.9 14.8Q2.9 13.6 4.1 13.6L9.1 13.6Q10.3 13.6 10.3 14.8L10.3 14.8Q10.3 16 9.1 16L4.1 16Q2.9 16 2.9 14.8ZM2.9 19.5Q2.9 18.3 4.1 18.3L9.1 18.3Q10.3 18.3 10.3 19.5L10.3 19.5Q10.3 20.7 9.1 20.7L4.1 20.7Q2.9 20.7 2.9 19.5Z"
private const val P4 = "M-11 -1Q-11 -1.8 -10.2 -1.8L-3.8 -1.8Q-3.6 -1.8 -3.421 -1.711L-0.358 -0.179Q0 0 -0.358 0.179L-3.421 1.711Q-3.6 1.8 -3.8 1.8L-10.2 1.8Q-11 1.8 -11 1Z"

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
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1f, endY = 24f))
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeAlpha = 0.55f, strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    addGroup(translationX = 17.6f, translationY = 15.6f)
    addGroup(rotate = 132f)
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Compose: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Compose",
        light = longArrayOf(0xFFD0926C, 0xFFC87E51, 0xFFE9CCBA, 0xFFDCAC8F, 0xFFB55215, 0xFF893400),
        dark = longArrayOf(0xFFDD9569, 0xFFE08B57, 0xFFD4B29D, 0xFFD9A281, 0xFFE7752F, 0xFFAE4E00),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
