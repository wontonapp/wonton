package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Pin: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Pin",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 235.32 81.37 L 174.63 20.69 a 16.0 16.0 0.0 0 0 -22.63 0.0 L 98.37 74.49 c -10.66 -3.34 -35.0 -7.37 -60.4 13.14 a 16.0 16.0 0.0 0 0 -1.29 23.78 L 85.0 159.71 L 42.34 202.34 a 8.0 8.0 0.0 0 0 11.32 11.32 L 96.29 171.0 l 48.29 48.29 A 16.0 16.0 0.0 0 0 155.9 224.0 c 0.38 0.0 0.75 0.0 1.13 0.0 a 15.93 15.93 0.0 0 0 11.64 -6.33 c 19.64 -26.1 17.75 -47.32 13.19 -60.0 L 235.33 104.0 A 16.0 16.0 0.0 0 0 235.32 81.37 Z M 224.0 92.69 h 0.0 l -57.27 57.46 a 8.0 8.0 0.0 0 0 -1.49 9.22 c 9.46 18.93 -1.8 38.59 -9.34 48.62 L 48.0 100.08 c 12.08 -9.74 23.64 -12.31 32.48 -12.31 A 40.13 40.13 0.0 0 1 96.81 91.0 a 8.0 8.0 0.0 0 0 9.25 -1.51 L 163.32 32.0 L 224.0 92.68 Z")
        )
    }.build()
}
