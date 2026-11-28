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

private const val P0 = "M5.2 4.4Q5.2 2.2 7.4 2.2L23.6 2.2Q25.8 2.2 25.8 4.4L25.8 20.6Q25.8 22.8 23.6 22.8L7.4 22.8Q5.2 22.8 5.2 20.6Z"
private const val P1 = "M7.6 5.8Q7.6 4.6 8.8 4.6L10.53 4.6Q11.73 4.6 11.73 5.8L11.73 7.53Q11.73 8.73 10.53 8.73L8.8 8.73Q7.6 8.73 7.6 7.53Z"
private const val P2 = "M13.43 5.8Q13.43 4.6 14.63 4.6L16.36 4.6Q17.56 4.6 17.56 5.8L17.56 7.53Q17.56 8.73 16.36 8.73L14.63 8.73Q13.43 8.73 13.43 7.53Z"
private const val P3 = "M19.26 5.8Q19.26 4.6 20.46 4.6L22.19 4.6Q23.39 4.6 23.39 5.8L23.39 7.53Q23.39 8.73 22.19 8.73L20.46 8.73Q19.26 8.73 19.26 7.53Z"
private const val P4 = "M7.6 11.63Q7.6 10.43 8.8 10.43L10.53 10.43Q11.73 10.43 11.73 11.63L11.73 13.36Q11.73 14.56 10.53 14.56L8.8 14.56Q7.6 14.56 7.6 13.36Z"
private const val P5 = "M13.43 11.63Q13.43 10.43 14.63 10.43L16.36 10.43Q17.56 10.43 17.56 11.63L17.56 13.36Q17.56 14.56 16.36 14.56L14.63 14.56Q13.43 14.56 13.43 13.36Z"
private const val P6 = "M19.26 11.63Q19.26 10.43 20.46 10.43L22.19 10.43Q23.39 10.43 23.39 11.63L23.39 13.36Q23.39 14.56 22.19 14.56L20.46 14.56Q19.26 14.56 19.26 13.36Z"
private const val P7 = "M7.6 17.46Q7.6 16.26 8.8 16.26L10.53 16.26Q11.73 16.26 11.73 17.46L11.73 19.19Q11.73 20.39 10.53 20.39L8.8 20.39Q7.6 20.39 7.6 19.19Z"
private const val P8 = "M13.43 17.46Q13.43 16.26 14.63 16.26L16.36 16.26Q17.56 16.26 17.56 17.46L17.56 19.19Q17.56 20.39 16.36 20.39L14.63 20.39Q13.43 20.39 13.43 19.19Z"
private const val P9 = "M19.26 17.46Q19.26 16.26 20.46 16.26L22.19 16.26Q23.39 16.26 23.39 17.46L23.39 19.19Q23.39 20.39 22.19 20.39L20.46 20.39Q19.26 20.39 19.26 19.19Z"

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
    addGroup(translationX = 9.665f, translationY = 6.665f)
    addGroup(translationX = -9.665f, translationY = -6.665f)
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 15.495f, translationY = 6.665f)
    addGroup(translationX = -15.495f, translationY = -6.665f)
    addPath(pathData = addPathNodes(P2), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 21.325f, translationY = 6.665f)
    addGroup(translationX = -21.325f, translationY = -6.665f)
    addPath(pathData = addPathNodes(P3), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 9.665f, translationY = 12.495f)
    addGroup(translationX = -9.665f, translationY = -12.495f)
    addPath(pathData = addPathNodes(P4), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 15.495f, translationY = 12.495f)
    addGroup(translationX = -15.495f, translationY = -12.495f)
    addPath(pathData = addPathNodes(P5), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 21.325f, translationY = 12.495f)
    addGroup(translationX = -21.325f, translationY = -12.495f)
    addPath(pathData = addPathNodes(P6), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 9.665f, translationY = 18.325f)
    addGroup(translationX = -9.665f, translationY = -18.325f)
    addPath(pathData = addPathNodes(P7), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 15.495f, translationY = 18.325f)
    addGroup(translationX = -15.495f, translationY = -18.325f)
    addPath(pathData = addPathNodes(P8), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    addGroup(translationX = 21.325f, translationY = 18.325f)
    addGroup(translationX = -21.325f, translationY = -18.325f)
    addPath(pathData = addPathNodes(P9), fill = SolidColor(Color(c[5])))
    clearGroup()
    clearGroup()
    clearGroup()
}.build()

val WontonIcon.Apps: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Apps",
        light = longArrayOf(0xFF8F9CF0, 0xFF7A8AED, 0xFFCAD0F8, 0xFFAAB4F3, 0xFF4D62E7, 0xFFFFFFFF),
        dark = longArrayOf(0xFF9CA5E1, 0xFF939EE5, 0xFFB5BAD6, 0xFFA7AEDC, 0xFF7F8EEE, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
