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
private const val P1 = "M5.388 -0.016L28.016 22.612L37.915 12.712L15.288 -9.915ZM2.984 2.388L25.612 25.016L15.712 34.915L-6.915 12.288Z"
private const val P2 = "M8.1 12.5A7.4 7.4 0 1 0 22.9 12.5A7.4 7.4 0 1 0 8.1 12.5Z"

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
    addGroup(clipPathData = addPathNodes(P1))
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Blocked: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Blocked",
        light = longArrayOf(0xFFE87276, 0xFFE3585C, 0xFFF4BDBE, 0xFFED9497, 0xFFDA1F25, 0xFFFFFFFF),
        dark = longArrayOf(0xFFDF9193, 0xFFE28689, 0xFFD5B0B1, 0xFFDA9FA1, 0xFFEA6E72, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
