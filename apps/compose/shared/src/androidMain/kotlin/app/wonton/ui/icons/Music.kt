package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Music: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Music",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 212.92 17.69 a 8.0 8.0 0.0 0 0 -6.86 -1.45 l -128.0 32.0 A 8.0 8.0 0.0 0 0 72.0 56.0 V 166.08 A 36.0 36.0 0.0 1 0 88.0 196.0 V 110.25 l 112.0 -28.0 v 51.83 A 36.0 36.0 0.0 1 0 216.0 164.0 V 24.0 A 8.0 8.0 0.0 0 0 212.92 17.69 Z M 52.0 216.0 a 20.0 20.0 0.0 1 1 20.0 -20.0 A 20.0 20.0 0.0 0 1 52.0 216.0 Z M 88.0 93.75 V 62.25 l 112.0 -28.0 v 31.5 Z M 180.0 184.0 a 20.0 20.0 0.0 1 1 20.0 -20.0 A 20.0 20.0 0.0 0 1 180.0 184.0 Z")
        )
    }.build()
}
