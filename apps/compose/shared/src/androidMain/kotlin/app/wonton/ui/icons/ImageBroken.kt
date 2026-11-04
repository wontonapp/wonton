package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.ImageBroken: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "ImageBroken",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 40.0 L 40.0 40.0 A 16.0 16.0 0.0 0 0 24.0 56.0 L 24.0 200.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 64.0 a 8.0 8.0 0.0 0 0 7.59 -5.47 l 14.83 -44.48 L 163.0 151.43 a 8.07 8.07 0.0 0 0 4.46 -4.46 l 14.62 -36.55 l 44.48 -14.83 A 8.0 8.0 0.0 0 0 232.0 88.0 L 232.0 56.0 A 16.0 16.0 0.0 0 0 216.0 40.0 Z M 112.41 157.47 L 98.23 200.0 L 40.0 200.0 L 40.0 172.0 l 52.0 -52.0 l 30.42 30.42 L 117.0 152.57 A 8.0 8.0 0.0 0 0 112.41 157.47 Z M 216.0 82.23 L 173.47 96.41 a 8.0 8.0 0.0 0 0 -4.9 4.62 l -14.72 36.82 L 138.58 144.0 l -35.27 -35.27 a 16.0 16.0 0.0 0 0 -22.62 0.0 L 40.0 149.37 L 40.0 56.0 L 216.0 56.0 Z M 228.68 115.23 a 8.0 8.0 0.0 0 0 -7.21 -1.1 l -23.8 7.94 a 8.0 8.0 0.0 0 0 -4.9 4.61 l -14.31 35.77 l -35.77 14.31 a 8.0 8.0 0.0 0 0 -4.61 4.9 l -7.94 23.8 A 8.0 8.0 0.0 0 0 137.73 216.0 L 216.0 216.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 232.0 121.73 A 8.0 8.0 0.0 0 0 228.68 115.24 Z M 216.0 200.0 L 148.83 200.0 l 3.25 -9.75 l 35.51 -14.2 a 8.07 8.07 0.0 0 0 4.46 -4.46 l 14.2 -35.51 l 9.75 -3.25 Z")
        )
    }.build()
}
