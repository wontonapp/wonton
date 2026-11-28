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

private const val P0 = "M12.026 4.34Q12.719 3.94 12.939 3.171L13.261 2.049Q13.481 1.28 14.281 1.28L16.719 1.28Q17.519 1.28 17.739 2.049L18.061 3.171Q18.281 3.94 18.974 4.34L20.829 5.412Q21.522 5.812 22.298 5.618L23.431 5.335Q24.207 5.142 24.607 5.835L25.826 7.946Q26.226 8.638 25.67 9.214L24.859 10.053Q24.303 10.629 24.303 11.429L24.303 13.571Q24.303 14.371 24.859 14.947L25.67 15.786Q26.226 16.362 25.826 17.054L24.607 19.165Q24.207 19.858 23.431 19.665L22.298 19.382Q21.522 19.188 20.829 19.588L18.974 20.66Q18.281 21.06 18.061 21.829L17.739 22.951Q17.519 23.72 16.719 23.72L14.281 23.72Q13.481 23.72 13.261 22.951L12.939 21.829Q12.719 21.06 12.026 20.66L10.171 19.588Q9.478 19.188 8.702 19.382L7.569 19.665Q6.793 19.858 6.393 19.165L5.174 17.054Q4.774 16.362 5.33 15.786L6.141 14.947Q6.697 14.371 6.697 13.571L6.697 11.429Q6.697 10.629 6.141 10.053L5.33 9.214Q4.774 8.638 5.174 7.946L6.393 5.835Q6.793 5.142 7.569 5.335L8.702 5.618Q9.478 5.812 10.171 5.412Z"
private const val P1 = "M9.7 12.5A5.8 5.8 0 1 0 21.3 12.5A5.8 5.8 0 1 0 9.7 12.5Z"
private const val P2 = "M12 12.5A3.5 3.5 0 1 0 19 12.5A3.5 3.5 0 1 0 12 12.5Z"

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 30.8571f,
    viewportHeight = 30.8571f,
    autoMirror = false,
).apply {
    addGroup(translationX = -0.0714f, translationY = 2.9286f)
    addPath(pathData = addPathNodes(P0), stroke = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.28f, endY = 23.72f), strokeLineWidth = 1f)
    addPath(pathData = addPathNodes(P0), stroke = SolidColor(Color(c[4])), strokeLineWidth = 2f)
    addPath(pathData = addPathNodes(P0), fill = Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1.28f, endY = 23.72f))
    addPath(pathData = addPathNodes(P1), fill = SolidColor(Color(c[5])))
    addPath(pathData = addPathNodes(P2), fill = Brush.verticalGradient(0f to Color(c[2]), 1f to Color(c[3]), startY = 9f, endY = 16f))
    clearGroup()
}.build()

val WontonIcon.Settings: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {
    WontonGlyph(
        name = "Settings",
        light = longArrayOf(0xFF969FAC, 0xFF828D9D, 0xFFCDD2D8, 0xFFAFB6C0, 0xFF58667B, 0xFFFFFFFF),
        dark = longArrayOf(0xFFA1AAB7, 0xFF99A4B3, 0xFFB8BCC2, 0xFFABB2BC, 0xFF8896AA, 0xFFFFFFFF),
        mono = longArrayOf(0xFF808080, 0xFF737373, 0xFFA1A1A1, 0xFF8F8F8F, 0xFF000000, 0xFFFFFFFF),
        build = ::art,
    )
}
