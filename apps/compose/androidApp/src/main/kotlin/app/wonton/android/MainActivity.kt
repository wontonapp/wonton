package app.wonton.android

import android.content.pm.ApplicationInfo
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import app.wonton.android.push.PushRegistration
import app.wonton.client.initLogging
import app.wonton.protocol.Action
import app.wonton.ui.StartupFailure
import app.wonton.ui.WontonApp
import app.wonton.ui.send
import java.io.File

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val debuggable = applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        if (debuggable) {
            // 标准错误不进 logcat：核心日志写到数据目录，用 adb shell run-as app.wonton.android cat files/core.log 看。
            val filter = intent.getStringExtra("wonton_log") ?: "wonton_client=info,wonton_tunnel=info,warn"
            initLogging(filter, File(filesDir, "core.log").path)
        }
        val core = (application as WontonApplication).core.getOrElse { failure ->
            setContent { StartupFailure("核心启动失败：$failure") }
            return
        }
        // 开发时用 adb 传入连接信息（dev.sh ticket 的输出），并在网页就绪后自动打开：
        // adb shell am start -n app.wonton.android/.MainActivity --es wonton_ticket '<JSON>' --ez wonton_open_web true
        val ticket = intent.getStringExtra("wonton_ticket")
        val rejected = if (debuggable && ticket != null) core.send(Action.ConnectHome(ticket)) else null
        if (rejected != null) {
            setContent { StartupFailure("连接信息被拒绝：$rejected") }
            return
        }
        val openWeb = debuggable && intent.getBooleanExtra("wonton_open_web", false)
        PushRegistration(this, core).start()
        setContent { WontonApp(core, openWebOnReady = openWeb) }
    }

    override fun onResume() {
        super.onResume()
        // 回到前台：网络可能变了，让核心检查隧道并重连。
        (application as WontonApplication).core.getOrNull()?.send(Action.AppForegrounded)
    }
}
