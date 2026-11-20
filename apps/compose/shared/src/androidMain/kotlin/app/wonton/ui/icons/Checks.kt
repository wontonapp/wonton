package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Checks: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Checks",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 149.61 85.71 l -89.6 88.0 a 8.0 8.0 0.0 0 1 -11.22 0.0 L 10.39 136.0 a 8.0 8.0 0.0 1 1 11.22 -11.41 L 54.4 156.79 l 84.0 -82.5 a 8.0 8.0 0.0 1 1 11.22 11.42 Z M 245.71 74.39 a 8.0 8.0 0.0 0 0 -11.32 -0.1 l -84.0 82.5 l -18.83 -18.5 a 8.0 8.0 0.0 0 0 -11.21 11.42 l 24.43 24.0 a 8.0 8.0 0.0 0 0 11.22 0.0 l 89.6 -88.0 A 8.0 8.0 0.0 0 0 245.71 74.39 Z")
        )
    }.build()
}
