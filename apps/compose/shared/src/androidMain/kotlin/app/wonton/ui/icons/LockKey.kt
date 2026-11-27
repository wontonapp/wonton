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

private const val P0 = "M7.5 9Q7.5 1 15.5 1L15.5 1Q23.5 1 23.5 9L23.5 16Q23.5 24 15.5 24L15.5 24Q7.5 24 7.5 16Z"
private const val P1 = "M10.5 12.5A5 5 0 1 0 20.5 12.5A5 5 0 1 0 10.5 12.5Z"
private const val P2 = "M13.7 11.2A1.8 1.8 0 1 0 17.3 11.2A1.8 1.8 0 1 0 13.7 11.2Z"
private const val P3 = "M14.6 12.3Q14.6 11.4 15.5 11.4L15.5 11.4Q16.4 11.4 16.4 12.3L16.4 14.7Q16.4 15.6 15.5 15.6L15.5 15.6Q14.6 15.6 14.6 14.7Z"

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
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 7.5f, endY = 17.5f))
    addGroup(translationX = 15.5f, translationY = 12.5f)
    addGroup(translationX = -15.5f, translationY = -12.5f)
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.LockKey: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "LockKey",
        light = longArrayOf(0xFF6CAF8B, 0xFF51A075, 0xFFBAD9C8, 0xFF8FC2A7, 0xFF158046, 0xFF005F34),
        dark = longArrayOf(0xFF5DB787, 0xFF49B37A, 0xFF98C2AB, 0xFF78BC97, 0xFF1CAB5E, 0xFF008047),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
