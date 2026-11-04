package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Device: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Device",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 176.0 16.0 L 80.0 16.0 A 24.0 24.0 0.0 0 0 56.0 40.0 L 56.0 216.0 a 24.0 24.0 0.0 0 0 24.0 24.0 h 96.0 a 24.0 24.0 0.0 0 0 24.0 -24.0 L 200.0 40.0 A 24.0 24.0 0.0 0 0 176.0 16.0 Z M 184.0 216.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 80.0 224.0 a 8.0 8.0 0.0 0 1 -8.0 -8.0 L 72.0 40.0 a 8.0 8.0 0.0 0 1 8.0 -8.0 h 96.0 a 8.0 8.0 0.0 0 1 8.0 8.0 Z M 140.0 60.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 140.0 60.0 Z")
        )
    }.build()
}
