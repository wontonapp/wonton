package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.VideoCall: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "VideoCall",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 251.77 73.0 a 8.0 8.0 0.0 0 0 -8.21 0.39 L 208.0 97.05 L 208.0 72.0 a 16.0 16.0 0.0 0 0 -16.0 -16.0 L 32.0 56.0 A 16.0 16.0 0.0 0 0 16.0 72.0 L 16.0 184.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 192.0 200.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 208.0 159.0 l 35.56 23.71 A 8.0 8.0 0.0 0 0 248.0 184.0 a 8.0 8.0 0.0 0 0 8.0 -8.0 L 256.0 80.0 A 8.0 8.0 0.0 0 0 251.77 73.0 Z M 192.0 184.0 L 32.0 184.0 L 32.0 72.0 L 192.0 72.0 L 192.0 184.0 Z M 240.0 161.05 L 208.0 139.72 L 208.0 116.28 L 240.0 95.0 Z")
        )
    }.build()
}
