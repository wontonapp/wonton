package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.GlobeFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "GlobeFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 24.0 h 0.0 A 104.0 104.0 0.0 1 0 232.0 128.0 A 104.12 104.12 0.0 0 0 128.0 24.0 Z M 206.36 88.0 L 170.71 88.0 a 135.28 135.28 0.0 0 0 -22.3 -45.6 A 88.29 88.29 0.0 0 1 206.37 88.0 Z M 216.0 128.0 a 87.61 87.61 0.0 0 1 -3.33 24.0 L 174.16 152.0 a 157.44 157.44 0.0 0 0 0.0 -48.0 h 38.51 A 87.61 87.61 0.0 0 1 216.0 128.0 Z M 128.0 43.0 a 115.27 115.27 0.0 0 1 26.0 45.0 L 102.0 88.0 A 115.11 115.11 0.0 0 1 128.0 43.0 Z M 102.0 168.0 L 154.0 168.0 a 115.11 115.11 0.0 0 1 -26.0 45.0 A 115.27 115.27 0.0 0 1 102.0 168.0 Z M 98.1 152.0 a 140.84 140.84 0.0 0 1 0.0 -48.0 h 59.88 a 140.84 140.84 0.0 0 1 0.0 48.0 Z M 148.45 213.6 a 135.28 135.28 0.0 0 0 22.3 -45.6 h 35.66 A 88.29 88.29 0.0 0 1 148.41 213.6 Z")
        )
    }.build()
}
