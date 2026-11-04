package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.WarningCircle: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "WarningCircle",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 24.0 A 104.0 104.0 0.0 1 0 232.0 128.0 A 104.11 104.11 0.0 0 0 128.0 24.0 Z M 128.0 216.0 a 88.0 88.0 0.0 1 1 88.0 -88.0 A 88.1 88.1 0.0 0 1 128.0 216.0 Z M 120.0 136.0 L 120.0 80.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 56.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 Z M 140.0 172.0 a 12.0 12.0 0.0 1 1 -12.0 -12.0 A 12.0 12.0 0.0 0 1 140.0 172.0 Z")
        )
    }.build()
}
