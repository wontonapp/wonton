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

private const val P0 = "M4.798 12.382Q3.8 11.4 5.101 10.882L25.285 2.844Q26.4 2.4 25.904 3.493L17.178 22.725Q16.6 24 15.602 23.018Z"
private const val P1 = "M25.904 3.493Q26.4 2.4 25.491 3.183L14.006 13.078Q13.4 13.6 13.635 14.365L16.188 22.662Q16.6 24 17.178 22.725Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = -0.4f, translationY = 0.2f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.7166f, endY = 23.4452f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 2.7166f, endY = 23.4452f))
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 2.857f, endY = 23.3468f))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Send: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Send",
        light = longArrayOf(0xFF679FF7, 0xFF4B8DF6, 0xFFB7D2FB, 0xFF8BB6F9, 0xFF0D66F3),
        dark = longArrayOf(0xFF82A9E7, 0xFF75A2EC, 0xFFA9BBD9, 0xFF94B1E0, 0xFF5794F7),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000),
        build = ::art,
    )
}
