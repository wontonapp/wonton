package app.wonton.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import app.wonton.ui.appearanceUpdates
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.video.VideoRoot

/** 只在 Debug 注册：用假数据核心直接打开影视页走查。影视页接进插件页之前，这是它在 App 里唯一的入口。 */
class VideoPreviewActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        intent.putExtra(UI_FAKE_EXTRA, true)
        (application as WontonApplication).configureForLaunch(intent)
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val core = (application as WontonApplication).core.getOrThrow()
        setContent {
            val updates = remember(core) { core.appearanceUpdates() }
            val appearance by updates.collectAsState(remember(core) { core.appearance() })
            WontonTheme(appearance) { VideoRoot(core) { finish() } }
        }
    }
}
