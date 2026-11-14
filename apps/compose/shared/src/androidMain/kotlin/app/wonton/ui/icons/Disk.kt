package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Disk: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Disk",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 64.0 L 32.0 64.0 A 16.0 16.0 0.0 0 0 16.0 80.0 v 96.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 224.0 192.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 240.0 80.0 A 16.0 16.0 0.0 0 0 224.0 64.0 Z M 224.0 176.0 L 32.0 176.0 L 32.0 80.0 L 224.0 80.0 v 96.0 Z M 200.0 128.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 200.0 128.0 Z")
        )
    }.build()
}
