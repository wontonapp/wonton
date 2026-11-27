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

private const val P0 = "M4 5.5Q4 3.3 6.2 3.3L24.8 3.3Q27 3.3 27 5.5L27 19.5Q27 21.7 24.8 21.7L6.2 21.7Q4 21.7 4 19.5Z"
private const val P1 = "M17.6 9.2A2.6 2.6 0 1 0 22.8 9.2A2.6 2.6 0 1 0 17.6 9.2Z"
private const val P2 = "M-1 23Q-1 23 -1 23L-1 20.4Q-1 20.4 -1 20.4L9.225 11.762Q10.6 10.6 11.979 11.757L23 21Q23 21 23 21L23 23Q23 23 23 23Z"
private const val P3 = "M8 23Q8 23 8 23L8 21.6Q8 21.6 8 21.6L18.334 14.244Q19.8 13.2 21.317 14.17L32 21Q32 21 32 21L32 23Q32 23 32 23Z"
private const val P4 = "M-5 0A5 5 0 1 0 5 0A5 5 0 1 0 -5 0Z"
private const val P5 = "M-1.9 -1.9L1.9 1.9M1.9 -1.9L-1.9 1.9"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 3.3f, endY = 21.7f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 3.3f, endY = 21.7f))
    addGroup(clipPathData = addPathNodes(P0))
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 11.1798f, endY = 23f))
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[4]), 1f to Color(c[4]), startY = 13.7035f, endY = 23f))
    clearGroup()
    addGroup(translationX = 22.6f, translationY = 18.4f)
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[6])))
    addPath(pathData = addPathNodes(P5), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.ImageBroken: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "ImageBroken",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFFFFFFFF, 0xFF33363D),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFFFFFFFF, 0xFF33363D),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF, 0xFF33363D),
        build = ::art,
    )
}
