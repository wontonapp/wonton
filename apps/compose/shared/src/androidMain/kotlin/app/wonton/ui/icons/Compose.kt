package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Compose: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Compose",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 229.66 58.34 l -32.0 -32.0 a 8.0 8.0 0.0 0 0 -11.32 0.0 l -96.0 96.0 A 8.0 8.0 0.0 0 0 88.0 128.0 v 32.0 a 8.0 8.0 0.0 0 0 8.0 8.0 h 32.0 a 8.0 8.0 0.0 0 0 5.66 -2.34 l 96.0 -96.0 A 8.0 8.0 0.0 0 0 229.66 58.34 Z M 124.69 152.0 H 104.0 V 131.31 l 64.0 -64.0 L 188.69 88.0 Z M 200.0 76.69 L 179.31 56.0 L 192.0 43.31 L 212.69 64.0 Z M 224.0 128.0 v 80.0 a 16.0 16.0 0.0 0 1 -16.0 16.0 H 48.0 a 16.0 16.0 0.0 0 1 -16.0 -16.0 V 48.0 A 16.0 16.0 0.0 0 1 48.0 32.0 h 80.0 a 8.0 8.0 0.0 0 1 0.0 16.0 H 48.0 V 208.0 H 208.0 V 128.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z")
        )
    }.build()
}
