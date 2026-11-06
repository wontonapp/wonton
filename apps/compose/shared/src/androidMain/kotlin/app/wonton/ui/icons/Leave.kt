package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Leave: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Leave",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 128.0 24.0 A 104.0 104.0 0.0 1 0 232.0 128.0 A 104.11 104.11 0.0 0 0 128.0 24.0 Z M 128.0 216.0 a 88.0 88.0 0.0 1 1 88.0 -88.0 A 88.1 88.1 0.0 0 1 128.0 216.0 Z M 128.0 56.0 a 72.0 72.0 0.0 1 0 72.0 72.0 A 72.08 72.08 0.0 0 0 128.0 56.0 Z M 128.0 184.0 a 56.0 56.0 0.0 1 1 56.0 -56.0 A 56.06 56.06 0.0 0 1 128.0 184.0 Z")
        )
    }.build()
}
