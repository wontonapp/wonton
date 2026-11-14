package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.VideoCallFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "VideoCallFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 192.0 72.0 L 192.0 184.0 a 16.0 16.0 0.0 0 1 -16.0 16.0 L 32.0 200.0 a 16.0 16.0 0.0 0 1 -16.0 -16.0 L 16.0 72.0 A 16.0 16.0 0.0 0 1 32.0 56.0 L 176.0 56.0 A 16.0 16.0 0.0 0 1 192.0 72.0 Z M 250.0 72.25 a 8.23 8.23 0.0 0 0 -6.63 1.22 L 209.78 95.86 A 4.0 4.0 0.0 0 0 208.0 99.19 v 57.62 a 4.0 4.0 0.0 0 0 1.78 3.33 l 33.78 22.52 a 8.0 8.0 0.0 0 0 8.58 0.19 a 8.33 8.33 0.0 0 0 3.86 -7.17 L 256.0 80.0 A 8.0 8.0 0.0 0 0 250.0 72.25 Z")
        )
    }.build()
}
