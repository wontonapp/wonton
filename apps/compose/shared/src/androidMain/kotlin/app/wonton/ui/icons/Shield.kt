package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Shield: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Shield",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 208.0 40.0 L 48.0 40.0 A 16.0 16.0 0.0 0 0 32.0 56.0 v 56.0 c 0.0 52.72 25.52 84.67 46.93 102.19 c 23.06 18.86 46.0 25.26 47.0 25.53 a 8.0 8.0 0.0 0 0 4.2 0.0 c 1.0 -0.27 23.91 -6.67 47.0 -25.53 C 198.48 196.67 224.0 164.72 224.0 112.0 L 224.0 56.0 A 16.0 16.0 0.0 0 0 208.0 40.0 Z M 208.0 112.0 c 0.0 37.07 -13.66 67.16 -40.6 89.42 A 129.3 129.3 0.0 0 1 128.0 223.62 a 128.25 128.25 0.0 0 1 -38.92 -21.81 C 61.82 179.51 48.0 149.3 48.0 112.0 l 0.0 -56.0 l 160.0 0.0 Z M 82.34 141.66 a 8.0 8.0 0.0 0 1 11.32 -11.32 L 112.0 148.69 l 50.34 -50.35 a 8.0 8.0 0.0 0 1 11.32 11.32 l -56.0 56.0 a 8.0 8.0 0.0 0 1 -11.32 0.0 Z")
        )
    }.build()
}
