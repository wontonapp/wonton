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

private const val P0 = "M5.2 4.4Q5.2 2.2 7.4 2.2L23.6 2.2Q25.8 2.2 25.8 4.4L25.8 20.6Q25.8 22.8 23.6 22.8L7.4 22.8Q5.2 22.8 5.2 20.6Z"
private const val P1 = "M8.3 0A1.5 1.5 0 1 0 11.3 0A1.5 1.5 0 1 0 8.3 0Z"
private const val P2 = "M12.8 0Q12.8 -1.2 14 -1.2L21 -1.2Q22.2 -1.2 22.2 0L22.2 0Q22.2 1.2 21 1.2L14 1.2Q12.8 1.2 12.8 0Z"
private const val P3 = "M12.8 0Q12.8 -1.2 14 -1.2L19.2 -1.2Q20.4 -1.2 20.4 0L20.4 0Q20.4 1.2 19.2 1.2L14 1.2Q12.8 1.2 12.8 0Z"
private const val P4 = "M12.8 0Q12.8 -1.2 14 -1.2L20.2 -1.2Q21.4 -1.2 21.4 0L21.4 0Q21.4 1.2 20.2 1.2L14 1.2Q12.8 1.2 12.8 0Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.2f, endY = 22.8f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.2f, endY = 22.8f))
    addGroup(translationX = 0f, translationY = 7.6f)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    clearGroup()
    addGroup(translationX = 0f, translationY = 12.5f)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    clearGroup()
    addGroup(translationX = 0f, translationY = 17.4f)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.List: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "List",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFFFFFFFF),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
