package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Connection: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Connection",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 88.0 a 40.0 40.0 0.0 1 0 40.0 40.0 A 40.0 40.0 0.0 0 0 128.0 88.0 Z M 128.0 152.0 a 24.0 24.0 0.0 1 1 24.0 -24.0 A 24.0 24.0 0.0 0 1 128.0 152.0 Z M 201.71 159.14 a 80.0 80.0 0.0 0 1 -14.08 22.2 a 8.0 8.0 0.0 0 1 -11.92 -10.67 a 63.95 63.95 0.0 0 0 0.0 -85.33 a 8.0 8.0 0.0 1 1 11.92 -10.67 a 80.08 80.08 0.0 0 1 14.08 84.47 Z M 69.0 103.09 a 64.0 64.0 0.0 0 0 11.26 67.58 a 8.0 8.0 0.0 0 1 -11.92 10.67 a 79.93 79.93 0.0 0 1 0.0 -106.67 A 8.0 8.0 0.0 1 1 80.29 85.34 A 63.77 63.77 0.0 0 0 69.0 103.09 Z M 248.0 128.0 a 119.58 119.58 0.0 0 1 -34.29 84.0 a 8.0 8.0 0.0 1 1 -11.42 -11.2 a 103.9 103.9 0.0 0 0 0.0 -145.56 A 8.0 8.0 0.0 1 1 213.71 44.0 A 119.58 119.58 0.0 0 1 248.0 128.0 Z M 53.71 200.78 A 8.0 8.0 0.0 1 1 42.29 212.0 a 119.87 119.87 0.0 0 1 0.0 -168.0 a 8.0 8.0 0.0 1 1 11.42 11.2 a 103.9 103.9 0.0 0 0 0.0 145.56 Z")
        )
    }.build()
}
