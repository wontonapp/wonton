package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Attach: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Attach",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 209.66 122.34 a 8.0 8.0 0.0 0 1 0.0 11.32 l -82.05 82.0 a 56.0 56.0 0.0 0 1 -79.2 -79.21 L 147.67 35.73 a 40.0 40.0 0.0 1 1 56.61 56.55 L 105.0 193.0 A 24.0 24.0 0.0 1 1 71.0 159.0 L 154.3 74.38 A 8.0 8.0 0.0 1 1 165.7 85.6 L 82.39 170.31 a 8.0 8.0 0.0 1 0 11.27 11.36 L 192.93 81.0 A 24.0 24.0 0.0 1 0 159.0 47.0 L 59.76 147.68 a 40.0 40.0 0.0 1 0 56.53 56.62 l 82.06 -82.0 A 8.0 8.0 0.0 0 1 209.66 122.34 Z")
        )
    }.build()
}
