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

private const val P0 = "M15.5 1.4L24.2 4.8V11.4C24.2 17.2 20.6 21.6 15.5 23.8C10.4 21.6 6.8 17.2 6.8 11.4V4.8Z"
private const val P1 = "M15.5 -2h16v30h-16Z"
private const val P2 = "M11.4 12.2L14.4 15.2L19.8 9.6"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 15.5f, translationY = 12.6f)
    addGroup(translationX = -15.5f, translationY = -12.6f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.4f, endY = 23.8f))
    addGroup(clipPathData = addPathNodes(P1))
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.4f, endY = 23.8f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 1.4f, endY = 23.8f))
    clearGroup()
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    clearGroup()
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Shield: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Shield",
        light = longArrayOf(0xFF6CAF8B, 0xFF51A075, 0xFFBAD9C8, 0xFF8FC2A7, 0xFF158046, 0xFF005F34),
        dark = longArrayOf(0xFF5DB787, 0xFF49B37A, 0xFF98C2AB, 0xFF78BC97, 0xFF1CAB5E, 0xFF008047),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000),
        build = ::art,
    )
}
