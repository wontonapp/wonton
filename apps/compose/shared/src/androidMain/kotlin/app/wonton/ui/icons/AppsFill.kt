package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.AppsFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "AppsFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 120.0 56.0 v 48.0 a 16.0 16.0 0.0 0 1 -16.0 16.0 L 56.0 120.0 a 16.0 16.0 0.0 0 1 -16.0 -16.0 L 40.0 56.0 A 16.0 16.0 0.0 0 1 56.0 40.0 h 48.0 A 16.0 16.0 0.0 0 1 120.0 56.0 Z M 200.0 40.0 L 152.0 40.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 48.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 216.0 56.0 A 16.0 16.0 0.0 0 0 200.0 40.0 Z M 104.0 136.0 L 56.0 136.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 48.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 120.0 152.0 A 16.0 16.0 0.0 0 0 104.0 136.0 Z M 200.0 136.0 L 152.0 136.0 a 16.0 16.0 0.0 0 0 -16.0 16.0 v 48.0 a 16.0 16.0 0.0 0 0 16.0 16.0 h 48.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 216.0 152.0 A 16.0 16.0 0.0 0 0 200.0 136.0 Z")
        )
    }.build()
}
