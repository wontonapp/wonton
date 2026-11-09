package app.wonton.media

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import android.webkit.WebView
import androidx.core.content.ContextCompat
import app.wonton.client.ClientCore
import app.wonton.client.PageMediaAction
import app.wonton.client.PageMediaPlayback
import app.wonton.client.PageMediaState
import app.wonton.ui.R
import java.lang.ref.WeakReference

private const val RETRY_START_MS = 5_000L
private const val TAG = "WontonMedia"

internal object PageMediaOwner {
    private var page = WeakReference<WebView>(null)
    private var core: ClientCore? = null
    private var starting = false
    private var keepingAlive = false
    private val handler = Handler(Looper.getMainLooper())
    private var lastStartFailure: String? = null
    private var bound: Context? = null
    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) = Unit
        override fun onServiceDisconnected(name: ComponentName) {
            bound = null
        }
    }
    private val retryStart = Runnable { state?.let { startIfPlaying(it) } }
    var state: PageMediaState? = null
        private set
    var service: PageMediaService? = null

    fun update(view: WebView, client: ClientCore, value: PageMediaState) {
        if (value.playback == PageMediaPlayback.PLAYING) {
            page = WeakReference(view)
            core = client
        } else if (page.get() !== view) {
            return
        }
        if (value.playback == PageMediaPlayback.NONE || value.playback == PageMediaPlayback.OTHER) {
            dispose(view)
            return
        }
        keepAlive(value.playback == PageMediaPlayback.PLAYING)
        state = value
        val running = service
        if (running != null) {
            running.update(value)
        } else {
            startIfPlaying(value)
        }
    }

    private fun startIfPlaying(value: PageMediaState) {
        val view = page.get() ?: return
        if (value.playback != PageMediaPlayback.PLAYING || service != null || starting) return
        starting = true
        val intent = Intent(view.context, PageMediaService::class.java)
        if (bound == null) {
            // 绑着它，暂停后系统才不会把服务（和会话）回收掉。
            bound = view.context.takeIf { it.bindService(intent, connection, Context.BIND_AUTO_CREATE) }
        }
        try {
            ContextCompat.startForegroundService(view.context, intent)
        } catch (foreground: IllegalStateException) {
            try {
                view.context.startService(intent)
            } catch (background: IllegalStateException) {
                // App 在后台时两种启动都可能被拒；播放和保活照常，服务稍后重试。
                starting = false
                val reason = background.message ?: background.javaClass.simpleName
                if (reason != lastStartFailure) {
                    lastStartFailure = reason
                    Log.w(TAG, view.context.getString(R.string.page_media_start_failed, reason, foreground.message))
                }
                handler.removeCallbacks(retryStart)
                handler.postDelayed(retryStart, RETRY_START_MS)
            }
        }
    }

    fun serviceStarted(value: PageMediaService) {
        starting = false
        lastStartFailure = null
        handler.removeCallbacks(retryStart)
        service = value
    }

    private fun keepAlive(active: Boolean) {
        if (keepingAlive == active) return
        keepingAlive = active
        core?.pageMediaActive(active)
    }

    fun command(action: PageMediaAction, seekMs: ULong? = null) {
        val view = page.get() ?: return
        val script = core?.pageMediaCommandScript(action, seekMs) ?: return
        view.evaluateJavascript(script, null)
    }

    fun dispose(view: WebView) {
        if (page.get() !== view) return
        handler.removeCallbacks(retryStart)
        keepAlive(false)
        view.context.let { context ->
            bound?.let { context.unbindService(connection) }
            bound = null
        }
        page.clear()
        core = null
        state = null
        starting = false
        service?.closeSession()
        view.context.stopService(Intent(view.context, PageMediaService::class.java))
    }
}
