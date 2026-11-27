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

private const val P0 = "M9.6 5.2Q9.6 3.6 11.2 3.6L15.4 3.6Q17 3.6 17 5.2L17 7.6Q17 7.6 17 7.6L9.6 7.6Q9.6 7.6 9.6 7.6Z"
private const val P1 = "M4 8.8Q4 6.6 6.2 6.6L24.8 6.6Q27 6.6 27 8.8L27 19.5Q27 21.7 24.8 21.7L6.2 21.7Q4 21.7 4 19.5Z"
private const val P2 = "M10.1 14.2A5.4 5.4 0 1 0 20.9 14.2A5.4 5.4 0 1 0 10.1 14.2Z"
private const val P3 = "M12.7 14.2A2.8 2.8 0 1 0 18.3 14.2A2.8 2.8 0 1 0 12.7 14.2Z"
private const val P4 = "M-13.6 0Q-13.6 -1.4 -12.2 -1.4L12.2 -1.4Q13.6 -1.4 13.6 0L13.6 0Q13.6 1.4 12.2 1.4L-12.2 1.4Q-13.6 1.4 -13.6 0Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.6f, endY = 7.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 3.6f, endY = 7.6f))
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 6.6f, endY = 21.7f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 6.6f, endY = 21.7f))
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 8.8f, endY = 19.6f))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    addGroup(translationX = 15.5f, translationY = 12.5f)
    addGroup(rotate = 45f)
    addPath(pathData = addPathNodes(P4), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -1.4f, endY = 1.4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P4), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P4), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -1.4f, endY = 1.4f))
    clearGroup()
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.CameraOff: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "CameraOff",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFF3E475B),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFF616B7E),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
