package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Folder: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Folder",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 72.0 H 131.31 L 104.0 44.69 A 15.86 15.86 0.0 0 0 92.69 40.0 H 40.0 A 16.0 16.0 0.0 0 0 24.0 56.0 V 200.62 A 15.4 15.4 0.0 0 0 39.38 216.0 H 216.89 A 15.13 15.13 0.0 0 0 232.0 200.89 V 88.0 A 16.0 16.0 0.0 0 0 216.0 72.0 Z M 40.0 56.0 H 92.69 l 16.0 16.0 H 40.0 Z M 216.0 200.0 H 40.0 V 88.0 H 216.0 Z")
        )
    }.build()
}
