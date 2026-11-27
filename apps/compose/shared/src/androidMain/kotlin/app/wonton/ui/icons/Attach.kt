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

private const val P0 = "M-1.2 -1L-1.2 7.011L-1.09 7.979L-0.764 8.909L-0.24 9.743L0.457 10.44L1.291 10.964L2.221 11.29L3.2 11.4L4.179 11.29L5.109 10.964L5.943 10.44L6.64 9.743L7.164 8.909L7.49 7.979L7.6 7.008L7.6 -3.904L7.552 -4.751L7.409 -5.591L7.174 -6.41L6.847 -7.198L6.435 -7.943L5.942 -8.639L5.374 -9.274L4.739 -9.842L4.043 -10.335L3.298 -10.747L2.51 -11.074L1.691 -11.309L0.851 -11.452L0 -11.5L-0.851 -11.452L-1.691 -11.309L-2.51 -11.074L-3.298 -10.747L-4.043 -10.335L-4.739 -9.842L-5.374 -9.274L-5.942 -8.639L-6.435 -7.943L-6.847 -7.198L-7.174 -6.41L-7.409 -5.591L-7.552 -4.751L-7.6 -3.905L-7.6 4.4A1.2 1.2 0 0 1 -5.2 4.4L-5.2 -3.895L-5.167 -4.482L-5.07 -5.057L-4.908 -5.617L-4.685 -6.156L-4.403 -6.667L-4.066 -7.142L-3.677 -7.577L-3.242 -7.966L-2.767 -8.303L-2.256 -8.585L-1.717 -8.808L-1.157 -8.97L-0.582 -9.067L0 -9.1L0.582 -9.067L1.157 -8.97L1.717 -8.808L2.256 -8.585L2.767 -8.303L3.242 -7.966L3.677 -7.577L4.066 -7.142L4.403 -6.667L4.685 -6.156L4.908 -5.617L5.07 -5.057L5.167 -4.482L5.2 -3.896L5.2 6.992L5.15 7.445L5.002 7.868L4.764 8.247L4.447 8.564L4.068 8.802L3.645 8.95L3.2 9L2.755 8.95L2.332 8.802L1.953 8.564L1.636 8.247L1.398 7.868L1.25 7.445L1.2 6.989L1.2 -1A1.2 1.2 0 0 1 -1.2 -1Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 15.5f, translationY = 12.5f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = -11.5f, endY = 11.4f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = -11.5f, endY = 11.4f))
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Attach: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Attach",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000),
        build = ::art,
    )
}
