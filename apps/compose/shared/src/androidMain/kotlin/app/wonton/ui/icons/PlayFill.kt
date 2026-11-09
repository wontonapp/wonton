package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.PlayFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "PlayFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 240.0 128.0 a 15.74 15.74 0.0 0 1 -7.6 13.51 L 88.32 229.65 a 16.0 16.0 0.0 0 1 -16.2 0.3 A 15.86 15.86 0.0 0 1 64.0 216.13 V 39.87 a 15.86 15.86 0.0 0 1 8.12 -13.82 a 16.0 16.0 0.0 0 1 16.2 0.3 L 232.4 114.49 A 15.74 15.74 0.0 0 1 240.0 128.0 Z")
        )
    }.build()
}
