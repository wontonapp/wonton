package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.SettingsFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "SettingsFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 237.94 107.21 a 8.0 8.0 0.0 0 0 -3.89 -5.4 l -29.83 -17.0 l -0.12 -33.62 a 8.0 8.0 0.0 0 0 -2.83 -6.08 a 111.91 111.91 0.0 0 0 -36.72 -20.67 a 8.0 8.0 0.0 0 0 -6.46 0.59 L 128.0 41.85 L 97.88 25.0 a 8.0 8.0 0.0 0 0 -6.47 -0.6 A 111.92 111.92 0.0 0 0 54.73 45.15 a 8.0 8.0 0.0 0 0 -2.83 6.07 l -0.15 33.65 l -29.83 17.0 a 8.0 8.0 0.0 0 0 -3.89 5.4 a 106.47 106.47 0.0 0 0 0.0 41.56 a 8.0 8.0 0.0 0 0 3.89 5.4 l 29.83 17.0 l 0.12 33.63 a 8.0 8.0 0.0 0 0 2.83 6.08 a 111.91 111.91 0.0 0 0 36.72 20.67 a 8.0 8.0 0.0 0 0 6.46 -0.59 L 128.0 214.15 L 158.12 231.0 a 7.91 7.91 0.0 0 0 3.9 1.0 a 8.09 8.09 0.0 0 0 2.57 -0.42 a 112.1 112.1 0.0 0 0 36.68 -20.73 a 8.0 8.0 0.0 0 0 2.83 -6.07 l 0.15 -33.65 l 29.83 -17.0 a 8.0 8.0 0.0 0 0 3.89 -5.4 A 106.47 106.47 0.0 0 0 237.94 107.21 Z M 128.0 168.0 a 40.0 40.0 0.0 1 1 40.0 -40.0 A 40.0 40.0 0.0 0 1 128.0 168.0 Z")
        )
    }.build()
}
