// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M4 4.2Q4 2 6.2 2L9.8 2Q11.2 2 12.19 2.99L13.21 4.01Q14.2 5 15.6 5L24.8 5Q27 5 27 7.2L27 20.8Q27 23 24.8 23L6.2 23Q4 23 4 20.8Z"
private const val P1 = "M0 1Q0 0 1 0L16 0Q17 0 17 1L17 8Q17 9 16 9L1 9Q0 9 0 8Z"
private const val P2 = "M5.5 2.3H11.5"
private const val P3 = "M4 7.2Q4 5 6.2 5L24.8 5Q27 5 27 7.2L27 20.8Q27 23 24.8 23L6.2 23Q4 23 4 20.8Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 0.943f to Color(c[4]), 0.943f to Color(c[5]), 1f to Color(c[6]), startY = 2f, endY = 23f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[9])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 0.943f to Color(c[4]), 0.943f to Color(c[5]), 1f to Color(c[6]), startY = 2f, endY = 23f))
    addGroup(translationX = 7f, translationY = 9f)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[10])))
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[11])), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round)
    clearGroup()
    addPath(pathData = addPathNodes(P3), stroke = Brush.verticalGradient(0f to Color(c[2]), 0.933f to Color(c[7]), 0.933f to Color(c[8]), 1f to Color(c[6]), startY = 5f, endY = 23f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P3), stroke = SolidColor(Color(c[9])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P3), fill = Brush.verticalGradient(0f to Color(c[2]), 0.933f to Color(c[7]), 0.933f to Color(c[8]), 1f to Color(c[6]), startY = 5f, endY = 23f))
    clearGroup()
}.build()

val WontonIcon.Folder: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Folder",
        light = longArrayOf(0xFFC09D64, 0xFFB48B47, 0xFFE1D1B6, 0xFFCFB489, 0xFFB68D4A, 0xFF8C5A07, 0xFFC3A16A, 0xFFD2B990, 0xFFBA9456, 0xFF9B6308, 0xFFFFFFFF, 0xFFE5D7BF),
        dark = longArrayOf(0xFFCE9F52, 0xFFCF963B, 0xFFCDB792, 0xFFCEAA6F, 0xFFCF973E, 0xFFBC7809, 0xFFCEA157, 0xFFCEAC75, 0xFFCF9B47, 0xFFD0840A, 0xFFFFFFFF, 0xFFCDB99A),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF444444, 0xFF000000, 0xFF656565, 0xFF8D8D8D, 0xFF505050, 0xFF000000, 0xFFFFFFFF, 0xFFBDBDBD),
        build = ::art,
    )
}
