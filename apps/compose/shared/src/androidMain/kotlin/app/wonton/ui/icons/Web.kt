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

private const val P0 = "M4 12.5A11.5 11.5 0 1 0 27 12.5A11.5 11.5 0 1 0 4 12.5Z"
private const val P1 = "M6.9 12.5A8.6 8.6 0 1 0 24.1 12.5A8.6 8.6 0 1 0 6.9 12.5Z"
private const val P2 = "M13.1 12.5Q13.1 12.5 13.1 12.5L15.295 6.464Q15.5 5.9 15.705 6.464L17.9 12.5Q17.9 12.5 17.9 12.5Z"
private const val P3 = "M17.9 12.5Q17.9 12.5 17.9 12.5L15.705 18.536Q15.5 19.1 15.295 18.536L13.1 12.5Q13.1 12.5 13.1 12.5Z"
private const val P4 = "M14.2 12.5A1.3 1.3 0 1 0 16.8 12.5A1.3 1.3 0 1 0 14.2 12.5Z"

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
    addGroup(rotate = 40f, pivotX = 15.5f, pivotY = 12.5f)
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[6])))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[7])))
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Web: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Web",
        light = longArrayOf(0xFF679FF7, 0xFF4B8DF6, 0xFFB7D2FB, 0xFF8BB6F9, 0xFF0D66F3, 0xFFFFFFFF, 0xFF2439C5, 0xFF5996F7),
        dark = longArrayOf(0xFF82A9E7, 0xFF75A2EC, 0xFFA9BBD9, 0xFF94B1E0, 0xFF5794F7, 0xFFFFFFFF, 0xFF4364C3, 0xFF7CA6EA),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF, 0xFF000000, 0xFF7A7A7A),
        build = ::art,
    )
}
