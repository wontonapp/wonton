package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.QrCode: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "QrCode",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 104.0 40.0 L 56.0 40.0 A 16.0 16.0 0.0 0 0 40.0 56.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 48.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 120.0 56.0 A 16.0 16.0 0.0 0 0 104.0 40.0 Z M 104.0 104.0 L 56.0 104.0 L 56.0 56.0 h 48.0 v 48.0 Z M 104.0 136.0 L 56.0 136.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 48.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 120.0 152.0 A 16.0 16.0 0.0 0 0 104.0 136.0 Z M 104.0 200.0 L 56.0 200.0 L 56.0 152.0 h 48.0 v 48.0 Z M 200.0 40.0 L 152.0 40.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 48.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 216.0 56.0 A 16.0 16.0 0.0 0 0 200.0 40.0 Z M 200.0 104.0 L 152.0 104.0 L 152.0 56.0 h 48.0 v 48.0 Z M 136.0 176.0 L 136.0 144.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 32.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 Z M 216.0 160.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 184.0 168.0 v 40.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 L 144.0 216.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 24.0 L 168.0 144.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 8.0 h 24.0 A 8.0 8.0 0.0 0 1 216.0 160.0 Z M 216.0 192.0 v 16.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 200.0 192.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z")
        )
    }.build()
}
