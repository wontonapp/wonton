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

private const val P0 = "M-0.08 -11.417Q0 -11.6 0.08 -11.417L3.094 -4.552Q3.174 -4.369 3.373 -4.349L10.833 -3.604Q11.032 -3.585 10.883 -3.452L5.285 1.536Q5.136 1.669 5.178 1.864L6.776 9.189Q6.818 9.385 6.646 9.284L0.173 5.501Q0 5.4 -0.173 5.501L-6.646 9.284Q-6.818 9.385 -6.776 9.189L-5.178 1.864Q-5.136 1.669 -5.285 1.536L-10.883 -3.452Q-11.032 -3.585 -10.833 -3.604L-3.373 -4.349Q-3.174 -4.369 -3.094 -4.552Z"
private const val P1 = "M-0.08 -5.417Q0 -5.6 0.08 -5.417L1.448 -2.287Q1.528 -2.103 1.727 -2.084L5.127 -1.75Q5.326 -1.73 5.176 -1.598L2.622 0.671Q2.473 0.803 2.516 0.999L3.249 4.335Q3.292 4.53 3.119 4.429L0.173 2.701Q0 2.6 -0.173 2.701L-3.119 4.429Q-3.292 4.53 -3.249 4.335L-2.516 0.999Q-2.473 0.803 -2.622 0.671L-5.176 -1.598Q-5.326 -1.73 -5.127 -1.75L-1.727 -2.084Q-1.528 -2.103 -1.448 -2.287Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 15.5f, translationY = 13.4f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = -11.5085f, endY = 9.3181f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = -11.5085f, endY = 9.3181f))
    clearGroup()
    addGroup(translationX = 15.5f, translationY = 13.4f)
    clearGroup()
    addGroup(translationX = 15.5f, translationY = 13.4f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -5.5085f, endY = 4.4632f))
    clearGroup()
    addGroup(translationX = 15.5f, translationY = 13.4f)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Star: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Star",
        light = longArrayOf(0xFFC09D64, 0xFFB48B47, 0xFFE1D1B6, 0xFFCFB489, 0xFF9B6308),
        dark = longArrayOf(0xFFCE9F52, 0xFFCF963B, 0xFFCDB792, 0xFFCEAA6F, 0xFFD0840A),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000),
        build = ::art,
    )
}
