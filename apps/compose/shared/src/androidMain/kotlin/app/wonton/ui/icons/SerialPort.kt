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

private const val P0 = "M15.5 18V23.6"
private const val P1 = "M11.3 3.4Q11.3 2.4 12.3 2.4L12.9 2.4Q13.9 2.4 13.9 3.4L13.9 7.8Q13.9 8.8 12.9 8.8L12.3 8.8Q11.3 8.8 11.3 7.8ZM17.1 3.4Q17.1 2.4 18.1 2.4L18.7 2.4Q19.7 2.4 19.7 3.4L19.7 7.8Q19.7 8.8 18.7 8.8L18.1 8.8Q17.1 8.8 17.1 7.8Z"
private const val P2 = "M7.8 9.6Q7.8 7.4 10 7.4L21 7.4Q23.2 7.4 23.2 9.6L23.2 9.8Q23.2 12.2 21.975 14.264L20.217 17.224Q19.4 18.6 17.8 18.6L13.2 18.6Q11.6 18.6 10.783 17.224L9.025 14.264Q7.8 12.2 7.8 9.8Z"
private const val P3 = "M16.49 10.185Q16.6 9.8 16.325 10.09L13.075 13.51Q12.8 13.8 13.2 13.8L14.8 13.8Q15.2 13.8 15.09 14.185L14.51 16.215Q14.4 16.6 14.675 16.31L17.925 12.89Q18.2 12.6 17.8 12.6L16.2 12.6Q15.8 12.6 15.91 12.215Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2.2f, strokeLineCap = StrokeCap.Round)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[4])))
    addPath(pathData = addPathNodes(P2), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 7.4f, endY = 18.6f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P2), stroke = SolidColor(Color(c[5])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 7.4f, endY = 18.6f))
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[6])), fillAlpha = 0.7f)
    clearGroup()
}.build()

val WontonIcon.SerialPort: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "SerialPort",
        light = longArrayOf(0xFFD0926C, 0xFFC87E51, 0xFFE9CCBA, 0xFFDCAC8F, 0xFF893400, 0xFFB55215, 0xFFFFFFFF),
        dark = longArrayOf(0xFFDD9569, 0xFFE08B57, 0xFFD4B29D, 0xFFD9A281, 0xFFAE4E00, 0xFFE7752F, 0xFFFFFFFF),
        mono = longArrayOf(0xFF5F5F5F, 0xFF414141, 0xFFB3B3B3, 0xFF858585, 0xFF000000, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
