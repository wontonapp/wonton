package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Refresh: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Refresh",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 48.0 V 96.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 H 168.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 28.69 L 182.06 73.37 a 79.56 79.56 0.0 0 0 -56.13 -23.43 h -0.45 A 79.52 79.52 0.0 0 0 69.59 72.71 A 8.0 8.0 0.0 0 1 58.41 61.27 a 96.0 96.0 0.0 0 1 135.0 0.79 L 208.0 76.69 V 48.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z M 186.41 183.29 a 80.0 80.0 0.0 0 1 -112.47 -0.66 L 59.31 168.0 H 88.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 H 40.0 a 8.0 8.0 0.0 0 0 -8.0 8.0 v 48.0 a 8.0 8.0 0.0 0 0 16.0 0.0 V 179.31 l 14.63 14.63 A 95.43 95.43 0.0 0 0 130.0 222.06 h 0.53 a 95.36 95.36 0.0 0 0 67.07 -27.33 a 8.0 8.0 0.0 0 0 -11.18 -11.44 Z")
        )
    }.build()
}
