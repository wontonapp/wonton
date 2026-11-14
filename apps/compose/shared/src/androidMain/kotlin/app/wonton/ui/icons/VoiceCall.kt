package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.VoiceCall: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "VoiceCall",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 222.37 158.46 l -47.11 -21.11 l -0.13 -0.06 a 16.0 16.0 0.0 0 0 -15.17 1.4 a 8.12 8.12 0.0 0 0 -0.75 0.56 L 134.87 160.0 c -15.42 -7.49 -31.34 -23.29 -38.83 -38.51 l 20.78 -24.71 c 0.2 -0.25 0.39 -0.5 0.57 -0.77 a 16.0 16.0 0.0 0 0 1.32 -15.06 l 0.0 -0.12 L 97.54 33.64 a 16.0 16.0 0.0 0 0 -16.62 -9.52 A 56.26 56.26 0.0 0 0 32.0 80.0 c 0.0 79.4 64.6 144.0 144.0 144.0 a 56.26 56.26 0.0 0 0 55.88 -48.92 A 16.0 16.0 0.0 0 0 222.37 158.46 Z M 176.0 208.0 A 128.14 128.14 0.0 0 1 48.0 80.0 A 40.2 40.2 0.0 0 1 82.87 40.0 a 0.61 0.61 0.0 0 0 0.0 0.12 l 21.0 47.0 L 83.2 111.86 a 6.13 6.13 0.0 0 0 -0.57 0.77 a 16.0 16.0 0.0 0 0 -1.0 15.7 c 9.06 18.53 27.73 37.06 46.46 46.11 a 16.0 16.0 0.0 0 0 15.75 -1.14 a 8.44 8.44 0.0 0 0 0.74 -0.56 L 168.89 152.0 l 47.0 21.05 h 0.0 s 0.08 0.0 0.11 0.0 A 40.21 40.21 0.0 0 1 176.0 208.0 Z")
        )
    }.build()
}
