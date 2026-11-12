package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Blocked: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Blocked",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 24.0 A 104.0 104.0 0.0 1 0 232.0 128.0 A 104.11 104.11 0.0 0 0 128.0 24.0 Z M 216.0 128.0 a 87.56 87.56 0.0 0 1 -20.41 56.28 L 71.72 60.4 A 88.0 88.0 0.0 0 1 216.0 128.0 Z M 40.0 128.0 A 87.56 87.56 0.0 0 1 60.41 71.72 L 184.28 195.6 A 88.0 88.0 0.0 0 1 40.0 128.0 Z")
        )
    }.build()
}
