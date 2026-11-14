package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Usb: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Usb",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 252.44 121.34 l -48.0 -32.0 A 8.0 8.0 0.0 0 0 192.0 96.0 v 24.0 L 72.0 120.0 L 72.0 72.0 h 33.0 a 32.0 32.0 0.0 1 0 0.0 -16.0 L 72.0 56.0 A 16.0 16.0 0.0 0 0 56.0 72.0 v 48.0 L 8.0 120.0 a 8.0 8.0 0.0 0 0 0.0 16.0 L 56.0 136.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 32.0 v 8.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 32.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 168.0 176.0 a 16.0 16.0 0.0 0 0 -16.0 -16.0 L 120.0 160.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 8.0 L 72.0 184.0 L 72.0 136.0 L 192.0 136.0 v 24.0 a 8.0 8.0 0.0 0 0 12.44 6.66 l 48.0 -32.0 a 8.0 8.0 0.0 0 0 0.0 -13.32 Z M 136.0 48.0 a 16.0 16.0 0.0 1 1 -16.0 16.0 A 16.0 16.0 0.0 0 1 136.0 48.0 Z M 120.0 176.0 h 32.0 v 32.0 L 120.0 208.0 Z M 208.0 145.05 L 208.0 111.0 l 25.58 17.0 Z")
        )
    }.build()
}
