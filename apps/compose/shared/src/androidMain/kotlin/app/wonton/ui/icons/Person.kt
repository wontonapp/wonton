package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Person: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Person",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 256.0 136.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 232.0 144.0 v 16.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 216.0 144.0 L 200.0 144.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 16.0 L 216.0 112.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 16.0 h 16.0 A 8.0 8.0 0.0 0 1 256.0 136.0 Z M 198.13 194.85 a 8.0 8.0 0.0 0 1 -12.26 10.3 C 165.75 181.19 138.09 168.0 108.0 168.0 s -57.75 13.19 -77.87 37.15 a 8.0 8.0 0.0 0 1 -12.25 -10.3 c 14.94 -17.78 33.52 -30.41 54.17 -37.17 a 68.0 68.0 0.0 1 1 71.9 0.0 C 164.6 164.44 183.18 177.07 198.13 194.85 Z M 108.0 152.0 a 52.0 52.0 0.0 1 0 -52.0 -52.0 A 52.06 52.06 0.0 0 0 108.0 152.0 Z")
        )
    }.build()
}
