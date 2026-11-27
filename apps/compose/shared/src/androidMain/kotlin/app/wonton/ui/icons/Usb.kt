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

private const val P0 = "M11.2 1.2Q11.2 0 12.4 0L18.6 0Q19.8 0 19.8 1.2L19.8 9Q19.8 9 19.8 9L11.2 9Q11.2 9 11.2 9Z"
private const val P1 = "M12.6 2.3Q12.6 1.8 13.1 1.8L14.5 1.8Q15 1.8 15 2.3L15 3.7Q15 4.2 14.5 4.2L13.1 4.2Q12.6 4.2 12.6 3.7ZM16 2.3Q16 1.8 16.5 1.8L17.9 1.8Q18.4 1.8 18.4 2.3L18.4 3.7Q18.4 4.2 17.9 4.2L16.5 4.2Q16 4.2 16 3.7Z"
private const val P2 = "M8 10.2Q8 8 10.2 8L20.8 8Q23 8 23 10.2L23 21.8Q23 24 20.8 24L10.2 24Q8 24 8 21.8Z"
private const val P3 = "M14.2 15.7Q14.2 14.4 15.5 14.4L15.5 14.4Q16.8 14.4 16.8 15.7L16.8 19.1Q16.8 20.4 15.5 20.4L15.5 20.4Q14.2 20.4 14.2 19.1Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 0f, translationY = 3.4f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 0f, endY = 9f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 0f, endY = 9f))
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    clearGroup()
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 8f, endY = 24f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 8f, endY = 24f))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[6])))
    clearGroup()
}.build()

val WontonIcon.Usb: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Usb",
        light = longArrayOf(0xFF8F9CF0, 0xFF7A8AED, 0xFFCAD0F8, 0xFFAAB4F3, 0xFF4D62E7, 0xFF4338B7, 0xFFFFFFFF),
        dark = longArrayOf(0xFF9CA5E1, 0xFF939EE5, 0xFFB5BAD6, 0xFFA7AEDC, 0xFF7F8EEE, 0xFF635FB8, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
