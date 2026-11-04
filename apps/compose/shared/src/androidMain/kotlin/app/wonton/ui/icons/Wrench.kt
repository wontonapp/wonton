package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Wrench: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Wrench",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 226.76 69.0 a 8.0 8.0 0.0 0 0 -12.84 -2.88 l -40.3 37.19 l -17.23 -3.7 l -3.7 -17.23 l 37.19 -40.3 A 8.0 8.0 0.0 0 0 187.0 29.24 A 72.0 72.0 0.0 0 0 88.0 96.0 A 72.34 72.34 0.0 0 0 94.0 124.94 L 33.79 177.0 c -0.15 0.12 -0.29 0.26 -0.43 0.39 a 32.0 32.0 0.0 0 0 45.26 45.26 c 0.13 -0.13 0.27 -0.28 0.39 -0.42 L 131.06 162.0 A 72.0 72.0 0.0 0 0 232.0 96.0 A 71.56 71.56 0.0 0 0 226.76 69.0 Z M 160.0 152.0 a 56.14 56.14 0.0 0 1 -27.07 -7.0 a 8.0 8.0 0.0 0 0 -9.92 1.77 L 67.11 211.51 a 16.0 16.0 0.0 0 1 -22.62 -22.62 L 109.18 133.0 a 8.0 8.0 0.0 0 0 1.77 -9.93 a 56.0 56.0 0.0 0 1 58.36 -82.31 l -31.2 33.81 a 8.0 8.0 0.0 0 0 -1.94 7.1 L 141.83 108.0 a 8.0 8.0 0.0 0 0 6.14 6.14 l 26.35 5.66 a 8.0 8.0 0.0 0 0 7.1 -1.94 l 33.81 -31.2 A 56.06 56.06 0.0 0 1 160.0 152.0 Z")
        )
    }.build()
}
