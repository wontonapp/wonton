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

private const val P0 = "M-0.158 -8.926Q0 -9.4 0.158 -8.926L0.78 -7.059Q0.783 -7.05 0.788 -7.041L1.875 -5.022Q1.88 -5.013 1.886 -5.006L3.284 -3.298Q3.29 -3.29 3.298 -3.284L5.006 -1.886Q5.013 -1.88 5.022 -1.875L7.041 -0.788Q7.05 -0.783 7.059 -0.78L8.926 -0.158Q9.4 0 8.926 0.158L7.059 0.78Q7.05 0.783 7.041 0.788L5.022 1.875Q5.013 1.88 5.006 1.886L3.298 3.284Q3.29 3.29 3.284 3.298L1.886 5.006Q1.88 5.013 1.875 5.022L0.788 7.041Q0.783 7.05 0.78 7.059L0.158 8.926Q0 9.4 -0.158 8.926L-0.78 7.059Q-0.783 7.05 -0.788 7.041L-1.875 5.022Q-1.88 5.013 -1.886 5.006L-3.284 3.298Q-3.29 3.29 -3.298 3.284L-5.006 1.886Q-5.013 1.88 -5.022 1.875L-7.041 0.788Q-7.05 0.783 -7.059 0.78L-8.926 0.158Q-9.4 0 -8.926 -0.158L-7.059 -0.78Q-7.05 -0.783 -7.041 -0.788L-5.022 -1.875Q-5.013 -1.88 -5.006 -1.886L-3.298 -3.284Q-3.29 -3.29 -3.284 -3.298L-1.886 -5.006Q-1.88 -5.013 -1.875 -5.022L-0.788 -7.041Q-0.783 -7.05 -0.78 -7.059Z"
private const val P1 = "M-0.158 -4.126Q0 -4.6 0.158 -4.126L0.38 -3.459Q0.383 -3.45 0.388 -3.441L0.915 -2.462Q0.92 -2.453 0.926 -2.446L1.604 -1.618Q1.61 -1.61 1.618 -1.604L2.446 -0.926Q2.453 -0.92 2.462 -0.915L3.441 -0.388Q3.45 -0.383 3.459 -0.38L4.126 -0.158Q4.6 0 4.126 0.158L3.459 0.38Q3.45 0.383 3.441 0.388L2.462 0.915Q2.453 0.92 2.446 0.926L1.618 1.604Q1.61 1.61 1.604 1.618L0.926 2.446Q0.92 2.453 0.915 2.462L0.388 3.441Q0.383 3.45 0.38 3.459L0.158 4.126Q0 4.6 -0.158 4.126L-0.38 3.459Q-0.383 3.45 -0.388 3.441L-0.915 2.462Q-0.92 2.453 -0.926 2.446L-1.604 1.618Q-1.61 1.61 -1.618 1.604L-2.446 0.926Q-2.453 0.92 -2.462 0.915L-3.441 0.388Q-3.45 0.383 -3.459 0.38L-4.126 0.158Q-4.6 0 -4.126 -0.158L-3.459 -0.38Q-3.45 -0.383 -3.441 -0.388L-2.462 -0.915Q-2.453 -0.92 -2.446 -0.926L-1.618 -1.604Q-1.61 -1.61 -1.604 -1.618L-0.926 -2.446Q-0.92 -2.453 -0.915 -2.462L-0.388 -3.441Q-0.383 -3.45 -0.38 -3.459Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 13.2f, translationY = 14.4f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = -9.163f, endY = 9.163f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = -9.163f, endY = 9.163f))
    clearGroup()
    addGroup(translationX = 13.2f, translationY = 14.4f)
    clearGroup()
    addGroup(translationX = 22.6f, translationY = 5.6f)
    addPath(pathData = addPathNodes(P1), stroke = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -4.363f, endY = 4.363f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P1), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P1), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = -4.363f, endY = 4.363f))
    clearGroup()
    addGroup(translationX = 22.6f, translationY = 5.6f)
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Agent: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Agent",
        light = longArrayOf(0xFFA892F5, 0xFF977EF3, 0xFFD6CCFA, 0xFFBDACF7, 0xFF7452EF),
        dark = longArrayOf(0xFFAE9FE5, 0xFFA896EA, 0xFFBEB7D8, 0xFFB5AADF, 0xFF9C84F4),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000),
        build = ::art,
    )
}
