package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Star: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Star",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 239.18 97.26 A 16.38 16.38 0.0 0 0 224.92 86.0 l -59.0 -4.76 L 143.14 26.15 a 16.36 16.36 0.0 0 0 -30.27 0.0 L 90.11 81.23 L 31.08 86.0 a 16.46 16.46 0.0 0 0 -9.37 28.86 l 45.0 38.83 L 53.0 211.75 a 16.38 16.38 0.0 0 0 24.5 17.82 L 128.0 198.49 l 50.53 31.08 A 16.4 16.4 0.0 0 0 203.0 211.75 l -13.76 -58.07 l 45.0 -38.83 A 16.43 16.43 0.0 0 0 239.18 97.26 Z M 223.84 102.73 L 175.14 144.73 a 8.0 8.0 0.0 0 0 -2.56 7.91 l 14.88 62.8 a 0.37 0.37 0.0 0 1 -0.17 0.48 c -0.18 0.14 -0.23 0.11 -0.38 0.0 l -54.72 -33.65 a 8.0 8.0 0.0 0 0 -8.38 0.0 L 69.09 215.94 c -0.15 0.09 -0.19 0.12 -0.38 0.0 a 0.37 0.37 0.0 0 1 -0.17 -0.48 l 14.88 -62.8 a 8.0 8.0 0.0 0 0 -2.56 -7.91 l -48.7 -42.0 c -0.12 -0.1 -0.23 -0.19 -0.13 -0.5 s 0.18 -0.27 0.33 -0.29 l 63.92 -5.16 A 8.0 8.0 0.0 0 0 103.0 91.86 l 24.62 -59.61 c 0.08 -0.17 0.11 -0.25 0.35 -0.25 s 0.27 0.08 0.35 0.25 L 153.0 91.86 a 8.0 8.0 0.0 0 0 6.75 4.92 l 63.92 5.16 c 0.15 0.0 0.24 0.0 0.33 0.29 S 224.0 102.63 223.84 102.73 Z")
        )
    }.build()
}
