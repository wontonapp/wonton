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

private const val P0 = "M23.943 19.924Q5.912 22.088 8.076 4.057L12.404 6.653Q13.269 14.731 21.347 15.596Z"
private const val P1 = "M25.242 22.665Q23.943 23.963 22.645 22.665L18.895 18.914Q17.596 17.616 18.895 16.318L21.058 14.154Q22.645 12.567 24.232 14.154L27.405 17.327Q28.992 18.914 27.405 20.501ZM11.682 9.105Q10.384 10.404 9.086 9.105L5.335 5.355Q4.037 4.057 5.335 2.758L7.499 0.595Q9.086 -0.992 10.673 0.595L13.846 3.768Q15.433 5.355 13.846 6.942Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 4.057f, endY = 20.1495f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 4.057f, endY = 20.1495f))
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -0.1985f, endY = 23.314f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -0.1985f, endY = 23.314f))
    clearGroup()
}.build()

val WontonIcon.VoiceCall: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "VoiceCall",
        light = longArrayOf(0xFF6CAF8B, 0xFF51A075, 0xFFBAD9C8, 0xFF8FC2A7, 0xFF158046),
        dark = longArrayOf(0xFF5DB787, 0xFF49B37A, 0xFF98C2AB, 0xFF78BC97, 0xFF1CAB5E),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000),
        build = ::art,
    )
}
