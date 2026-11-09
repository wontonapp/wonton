package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.LockKey: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "LockKey",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 112.0 a 28.0 28.0 0.0 0 0 -8.0 54.83 L 120.0 184.0 a 8.0 8.0 0.0 0 0 16.0 0.0 L 136.0 166.83 A 28.0 28.0 0.0 0 0 128.0 112.0 Z M 128.0 152.0 a 12.0 12.0 0.0 1 1 12.0 -12.0 A 12.0 12.0 0.0 0 1 128.0 152.0 Z M 208.0 80.0 L 176.0 80.0 L 176.0 56.0 a 48.0 48.0 0.0 0 0 -96.0 0.0 L 80.0 80.0 L 48.0 80.0 A 16.0 16.0 0.0 0 0 32.0 96.0 L 32.0 208.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 208.0 224.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 224.0 96.0 A 16.0 16.0 0.0 0 0 208.0 80.0 Z M 96.0 56.0 a 32.0 32.0 0.0 0 1 64.0 0.0 L 160.0 80.0 L 96.0 80.0 Z M 208.0 208.0 L 48.0 208.0 L 48.0 96.0 L 208.0 96.0 L 208.0 208.0 Z")
        )
    }.build()
}
