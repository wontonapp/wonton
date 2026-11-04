package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.FolderFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "FolderFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 72.0 H 131.31 L 104.0 44.69 A 15.88 15.88 0.0 0 0 92.69 40.0 H 40.0 A 16.0 16.0 0.0 0 0 24.0 56.0 V 200.62 A 15.41 15.41 0.0 0 0 39.39 216.0 h 177.5 A 15.13 15.13 0.0 0 0 232.0 200.89 V 88.0 A 16.0 16.0 0.0 0 0 216.0 72.0 Z M 40.0 56.0 H 92.69 l 16.0 16.0 H 40.0 Z")
        )
    }.build()
}
