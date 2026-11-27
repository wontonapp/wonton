// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M5.2 4.4Q5.2 2.2 7.4 2.2L23.6 2.2Q25.8 2.2 25.8 4.4L25.8 20.6Q25.8 22.8 23.6 22.8L7.4 22.8Q5.2 22.8 5.2 20.6Z"
private const val P1 = "M8 7.3Q8 6.1 9.2 6.1L11.4 6.1Q12.6 6.1 12.6 7.3L12.6 9.5Q12.6 10.7 11.4 10.7L9.2 10.7Q8 10.7 8 9.5Z"
private const val P2 = "M-1.5 0.1L-0.4 1.2L1.6 -1.1"
private const val P3 = "M14.8 8.4Q14.8 7.2 16 7.2L22.2 7.2Q23.4 7.2 23.4 8.4L23.4 8.4Q23.4 9.6 22.2 9.6L16 9.6Q14.8 9.6 14.8 8.4Z"
private const val P4 = "M8 15.5Q8 14.3 9.2 14.3L11.4 14.3Q12.6 14.3 12.6 15.5L12.6 17.7Q12.6 18.9 11.4 18.9L9.2 18.9Q8 18.9 8 17.7Z"
private const val P5 = "M14.8 16.6Q14.8 15.4 16 15.4L20.4 15.4Q21.6 15.4 21.6 16.6L21.6 16.6Q21.6 17.8 20.4 17.8L16 17.8Q14.8 17.8 14.8 16.6Z"

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
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addGroup(translationX = 10.3f, translationY = 8.4f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[6])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    clearGroup()
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P5), fill = SolidColor(Color(c[5])))
    clearGroup()
}.build()

val WontonIcon.Tasks: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Tasks",
        light = longArrayOf(0xFF6CAF8B, 0xFF51A075, 0xFFBAD9C8, 0xFF8FC2A7, 0xFF158046, 0xFFFFFFFF, 0xFF005F34),
        dark = longArrayOf(0xFF5DB787, 0xFF49B37A, 0xFF98C2AB, 0xFF78BC97, 0xFF1CAB5E, 0xFFFFFFFF, 0xFF008047),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFFFFFFFF, 0xFF000000),
        build = ::art,
    )
}
