package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Web: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Web",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 24.0 A 104.0 104.0 0.0 1 0 232.0 128.0 A 104.11 104.11 0.0 0 0 128.0 24.0 Z M 128.0 216.0 a 88.0 88.0 0.0 1 1 88.0 -88.0 A 88.1 88.1 0.0 0 1 128.0 216.0 Z M 172.42 72.84 l -64.0 32.0 a 8.05 8.05 0.0 0 0 -3.58 3.58 l -32.0 64.0 A 8.0 8.0 0.0 0 0 80.0 184.0 a 8.1 8.1 0.0 0 0 3.58 -0.84 l 64.0 -32.0 a 8.05 8.05 0.0 0 0 3.58 -3.58 l 32.0 -64.0 a 8.0 8.0 0.0 0 0 -10.74 -10.74 Z M 138.0 138.0 L 97.89 158.11 L 118.0 118.0 l 40.15 -20.07 Z")
        )
    }.build()
}
