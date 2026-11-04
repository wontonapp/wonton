package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Play: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Play",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 232.4 114.49 L 88.32 26.35 a 16.0 16.0 0.0 0 0 -16.2 -0.3 A 15.86 15.86 0.0 0 0 64.0 39.87 V 216.13 A 15.94 15.94 0.0 0 0 80.0 232.0 a 16.07 16.07 0.0 0 0 8.36 -2.35 L 232.4 141.51 a 15.81 15.81 0.0 0 0 0.0 -27.0 Z M 80.0 215.94 V 40.0 l 143.83 88.0 Z")
        )
    }.build()
}
