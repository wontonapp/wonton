@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.ui.video

import android.app.PendingIntent
import android.content.Intent
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.DefaultMediaNotificationProvider
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import app.wonton.ui.R

/**
 * 影视的播放器（ExoPlayer）和它的媒体会话。放在服务里：锁屏、切到画中画以外的地方声音照常，系统通知和耳机键能控制，
 * 播放时是前台服务，进程和隧道不会被掐。界面 [VideoPlayerActivity] 用 `MediaController` 连它。
 */
class VideoPlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val provider = DefaultMediaNotificationProvider.Builder(this)
            .setChannelName(R.string.video_playback_channel)
            .build()
        provider.setSmallIcon(android.R.drawable.ic_media_play)
        setMediaNotificationProvider(provider)
        val audio = AudioAttributes.Builder()
            .setUsage(C.USAGE_MEDIA)
            .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE)
            .build()
        val player = ExoPlayer.Builder(this)
            // 交给 ExoPlayer 管音频焦点：来电、别的 App 出声时暂停；拔耳机暂停。
            .setAudioAttributes(audio, true)
            .setHandleAudioBecomingNoisy(true)
            // 锁屏后从回环取流也不断：要 Wi-Fi 锁和唤醒锁。
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        val open = Intent(this, VideoPlayerActivity::class.java)
        session = MediaSession.Builder(this, player)
            .setSessionActivity(PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE))
            .build()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onDestroy() {
        session?.let { current ->
            current.player.release()
            current.release()
        }
        session = null
        super.onDestroy()
    }
}
