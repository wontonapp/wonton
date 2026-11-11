package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.MusicFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "MusicFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 212.92 17.71 a 7.89 7.89 0.0 0 0 -6.86 -1.46 l -128.0 32.0 A 8.0 8.0 0.0 0 0 72.0 56.0 V 166.1 A 36.0 36.0 0.0 1 0 88.0 196.0 V 102.25 l 112.0 -28.0 V 134.1 A 36.0 36.0 0.0 1 0 216.0 164.0 V 24.0 A 8.0 8.0 0.0 0 0 212.92 17.71 Z")
        )
    }.build()
}
