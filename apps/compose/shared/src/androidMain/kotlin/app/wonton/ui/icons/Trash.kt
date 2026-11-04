package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Trash: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Trash",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 48.0 L 176.0 48.0 L 176.0 40.0 a 24.0 24.0 0.0 0 0 -24.0 -24.0 L 104.0 16.0 A 24.0 24.0 0.0 0 0 80.0 40.0 v 8.0 L 40.0 48.0 a 8.0 8.0 0.0 0 0 0.0 16.0 h 8.0 L 48.0 208.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 192.0 224.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 208.0 64.0 h 8.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 Z M 96.0 40.0 a 8.0 8.0 0.0 0 1 8.0 -8.0 h 48.0 a 8.0 8.0 0.0 0 1 8.0 8.0 v 8.0 L 96.0 48.0 Z M 192.0 208.0 L 64.0 208.0 L 64.0 64.0 L 192.0 64.0 Z M 112.0 104.0 v 64.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 96.0 104.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z M 160.0 104.0 v 64.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 L 144.0 104.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z")
        )
    }.build()
}
