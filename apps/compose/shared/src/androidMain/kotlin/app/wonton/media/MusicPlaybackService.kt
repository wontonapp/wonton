@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.media

import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.Player
import androidx.media3.session.DefaultMediaNotificationProvider
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import app.wonton.ui.R

/** 和网页媒体的会话、通知分开：两边同时有声音时各占一条。 */
private const val SESSION_ID = "music"
private const val CHANNEL_ID = "music"
private const val NOTIFICATION_ID = 2_001
private const val TAG = "WontonMusic"

/** 通知栏、锁屏、耳机和车机上的那一块：放的是 [MusicPlayback] 的播放器，按钮交回它。 */
class MusicPlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val player = MusicPlayback.sessionPlayer()
        if (player == null) {
            stopSelf()
            return
        }
        val provider = DefaultMediaNotificationProvider.Builder(this)
            .setChannelId(CHANNEL_ID)
            .setChannelName(R.string.music_now_playing)
            .setNotificationId(NOTIFICATION_ID)
            .build()
        provider.setSmallIcon(android.R.drawable.ic_media_play)
        setMediaNotificationProvider(provider)
        val builder = MediaSession.Builder(this, player).setId(SESSION_ID)
        packageManager.getLaunchIntentForPackage(packageName)?.let { intent ->
            builder.setSessionActivity(
                PendingIntent.getActivity(
                    this, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
        }
        session = builder.build().also { addSession(it) }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onDestroy() {
        session?.let {
            removeSession(it)
            it.release()
        }
        session = null
        super.onDestroy()
    }

    internal companion object {
        private var bound: Context? = null
        private val connection = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, binder: IBinder) = Unit
            override fun onServiceDisconnected(name: ComponentName) {
                bound = null
            }
        }

        /** 开始出声：拉起服务，通知栏和锁屏才有这一块，进了后台也不被掐。 */
        fun start(context: Context) {
            val intent = Intent(context, MusicPlaybackService::class.java)
            if (bound == null) {
                // 绑着它，暂停后系统才不会把服务（和会话）回收掉。
                bound = context.takeIf { it.bindService(intent, connection, Context.BIND_AUTO_CREATE) }
            }
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (refused: IllegalStateException) {
                // 后台时系统可能不让起前台服务；声音照放，绑着的服务仍在，只是可能没有通知。
                Log.w(TAG, "foreground service refused: ${refused.message}")
            }
        }

        /** 会话停了：通知收掉、服务退出。 */
        fun stop(context: Context) {
            // 服务被系统回收过，绑定可能已经失效：解绑失败不挡下面的停服务。
            bound?.let { runCatching { context.unbindService(connection) } }
            bound = null
            context.stopService(Intent(context, MusicPlaybackService::class.java))
        }
    }
}

/** 上一首、下一首交给核心换（队列在核心）；别的照 ExoPlayer。 */
internal class MusicSessionPlayer(player: Player) : ForwardingPlayer(player) {
    override fun getAvailableCommands(): Player.Commands =
        super.getAvailableCommands().buildUpon().addAll(*NAVIGATION).build()

    override fun isCommandAvailable(command: Int): Boolean = command in NAVIGATION || super.isCommandAvailable(command)

    override fun seekToNext() = MusicPlayback.next()

    override fun seekToNextMediaItem() = MusicPlayback.next()

    override fun seekToPrevious() = MusicPlayback.previous()

    override fun seekToPreviousMediaItem() = MusicPlayback.previous()

    private companion object {
        val NAVIGATION = intArrayOf(
            Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
            Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
        )
    }
}
