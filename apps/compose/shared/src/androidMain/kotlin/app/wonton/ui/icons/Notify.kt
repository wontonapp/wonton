package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Notify: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Notify",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 71.1 a 8.0 8.0 0.0 0 1 -10.78 -3.42 a 94.13 94.13 0.0 0 0 -33.46 -36.91 a 8.0 8.0 0.0 1 1 8.54 -13.54 a 111.46 111.46 0.0 0 1 39.12 43.09 A 8.0 8.0 0.0 0 1 224.0 71.1 Z M 35.71 72.0 a 8.0 8.0 0.0 0 0 7.1 -4.32 A 94.13 94.13 0.0 0 1 76.27 30.77 a 8.0 8.0 0.0 1 0 -8.54 -13.54 A 111.46 111.46 0.0 0 0 28.61 60.32 A 8.0 8.0 0.0 0 0 35.71 72.0 Z M 221.81 175.94 A 16.0 16.0 0.0 0 1 208.0 200.0 L 167.2 200.0 a 40.0 40.0 0.0 0 1 -78.4 0.0 L 48.0 200.0 a 16.0 16.0 0.0 0 1 -13.79 -24.06 C 43.22 160.39 48.0 138.28 48.0 112.0 a 80.0 80.0 0.0 0 1 160.0 0.0 C 208.0 138.27 212.78 160.38 221.81 175.94 Z M 150.62 200.0 L 105.38 200.0 a 24.0 24.0 0.0 0 0 45.24 0.0 Z M 208.0 184.0 c -10.64 -18.27 -16.0 -42.49 -16.0 -72.0 a 64.0 64.0 0.0 0 0 -128.0 0.0 c 0.0 29.52 -5.38 53.74 -16.0 72.0 Z")
        )
    }.build()
}
