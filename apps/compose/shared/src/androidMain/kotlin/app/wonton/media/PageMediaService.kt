@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.media

import android.app.PendingIntent
import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.net.wifi.WifiManager
import android.os.PowerManager
import androidx.core.content.ContextCompat
import androidx.media3.session.DefaultMediaNotificationProvider
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import app.wonton.client.PageMediaAction
import app.wonton.client.PageMediaPlayback
import app.wonton.client.PageMediaState

/** 暂停后保持前台服务的时间，和核心的保活宽限一致。 */
private const val FOREGROUND_GRACE_MS = 60_000L

class PageMediaService : MediaSessionService() {
    private var session: MediaSession? = null
    private var player: PageMediaPlayer? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var receivingNoisy = false
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                PageMediaOwner.command(PageMediaAction.PAUSE)
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        // 暂停后保持前台服务 60 秒：进程留在前台服务里，Android 才不会掐掉隧道。
        setForegroundServiceTimeoutMs(FOREGROUND_GRACE_MS)
        val provider = DefaultMediaNotificationProvider.Builder(this)
            .setChannelName(app.wonton.ui.R.string.page_media_channel)
            .build()
        provider.setSmallIcon(android.R.drawable.stat_notify_chat)
        setMediaNotificationProvider(provider)
        val mediaPlayer = PageMediaPlayer(PageMediaOwner::command)
        player = mediaPlayer
        val builder = MediaSession.Builder(this, mediaPlayer)
        packageManager.getLaunchIntentForPackage(packageName)?.let { intent ->
            builder.setSessionActivity(PendingIntent.getActivity(this, 0, intent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        }
        session = builder.build().also { addSession(it) }
        PageMediaOwner.serviceStarted(this)
        val state = PageMediaOwner.state
        if (state != null) update(state) else stopSelf()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    internal fun update(state: PageMediaState) {
        val playing = state.playback == PageMediaPlayback.PLAYING
        updateLocks(playing)
        updateNoisyReceiver(playing)
        player?.update(state)
    }

    @Suppress("DEPRECATION")
    private fun updateLocks(playing: Boolean) {
        if (playing) {
            if (wakeLock == null) {
                wakeLock = getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Wonton:PageMedia")
                    .also { it.acquire() }
            }
            if (wifiLock == null) {
                wifiLock = getSystemService(WifiManager::class.java)
                    .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "Wonton:PageMedia")
                    .also { it.acquire() }
            }
        } else {
            wakeLock?.release()
            wakeLock = null
            wifiLock?.release()
            wifiLock = null
        }
    }

    private fun updateNoisyReceiver(playing: Boolean) {
        if (playing == receivingNoisy) return
        if (playing) {
            ContextCompat.registerReceiver(this, noisy,
                IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), ContextCompat.RECEIVER_EXPORTED)
        } else {
            unregisterReceiver(noisy)
        }
        receivingNoisy = playing
    }

    internal fun closeSession() {
        updateLocks(false)
        updateNoisyReceiver(false)
        session?.let { removeSession(it); it.release() }
        session = null
        player?.release()
        player = null
        stopForeground(STOP_FOREGROUND_REMOVE)
        getSystemService(NotificationManager::class.java)
            .cancel(DefaultMediaNotificationProvider.DEFAULT_NOTIFICATION_ID)
    }

    override fun onDestroy() {
        closeSession()
        if (PageMediaOwner.service === this) PageMediaOwner.service = null
        super.onDestroy()
    }
}
