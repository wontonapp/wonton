package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.CameraOff: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "CameraOff",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 53.92 34.62 A 8.0 8.0 0.0 1 0 42.08 45.38 L 51.73 56.0 L 48.0 56.0 A 24.0 24.0 0.0 0 0 24.0 80.0 L 24.0 192.0 a 24.0 24.0 0.0 0 0 24.0 24.0 L 197.19 216.0 l 4.89 5.38 a 8.0 8.0 0.0 1 0 11.84 -10.76 Z M 105.58 115.23 L 142.58 155.92 A 27.71 27.71 0.0 0 1 128.0 160.0 a 28.0 28.0 0.0 0 1 -22.42 -44.77 Z M 48.0 200.0 a 8.0 8.0 0.0 0 1 -8.0 -8.0 L 40.0 80.0 a 8.0 8.0 0.0 0 1 8.0 -8.0 L 66.28 72.0 l 28.41 31.26 A 44.0 44.0 0.0 0 0 128.0 176.0 a 44.21 44.21 0.0 0 0 25.44 -8.12 L 182.64 200.0 Z M 232.0 80.0 L 232.0 186.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 216.0 80.0 a 8.0 8.0 0.0 0 0 -8.0 -8.0 L 176.0 72.0 a 8.0 8.0 0.0 0 1 -6.65 -3.56 L 155.71 48.0 L 100.24 48.0 a 8.0 8.0 0.0 0 1 -12.91 -9.42 l 2.0 -3.0 A 8.0 8.0 0.0 0 1 96.0 32.0 h 64.0 a 8.0 8.0 0.0 0 1 6.66 3.56 L 180.28 56.0 L 208.0 56.0 A 24.0 24.0 0.0 0 1 232.0 80.0 Z")
        )
    }.build()
}
