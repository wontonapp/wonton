package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Server: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Server",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 208.0 136.0 L 48.0 136.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 208.0 216.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 224.0 152.0 A 16.0 16.0 0.0 0 0 208.0 136.0 Z M 208.0 200.0 L 48.0 200.0 L 48.0 152.0 L 208.0 152.0 v 48.0 Z M 208.0 40.0 L 48.0 40.0 A 16.0 16.0 0.0 0 0 32.0 56.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 208.0 120.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 224.0 56.0 A 16.0 16.0 0.0 0 0 208.0 40.0 Z M 208.0 104.0 L 48.0 104.0 L 48.0 56.0 L 208.0 56.0 v 48.0 Z M 192.0 80.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 192.0 80.0 Z M 192.0 176.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 192.0 176.0 Z")
        )
    }.build()
}
