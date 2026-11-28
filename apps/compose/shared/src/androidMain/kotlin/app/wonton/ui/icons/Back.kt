// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonGlyph

private const val P0 = "M-2 -5.4L2.4 0L-2 5.4"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = true,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addGroup(translationX = 15.5f, translationY = 12.5f)
    addGroup(scaleX = 1.5f, scaleY = 1.5f)
    addGroup(translationX = -15.5f, translationY = -12.5f)
    addGroup(translationX = 15.5f, translationY = 12.5f)
    addGroup(rotate = 180f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[0])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    clearGroup()
    clearGroup()
    clearGroup()
    clearGroup()
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Back: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Back",
        light = longArrayOf(0xFF58667B),
        dark = longArrayOf(0xFF8896AA),
        mono = longArrayOf(0xFF000000),
        build = ::art,
    )
}
