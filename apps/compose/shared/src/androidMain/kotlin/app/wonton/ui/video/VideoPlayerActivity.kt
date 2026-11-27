@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.ui.video

import android.app.PictureInPictureParams
import android.content.ComponentName
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.util.Rational
import androidx.activity.ComponentActivity
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import androidx.media3.ui.PlayerView
import com.google.common.util.concurrent.ListenableFuture

/** 画中画小窗允许的最扁、最窄的宽高比（系统的限制），超出的片子按边界算。 */
private const val PIP_MIN_RATIO = 0.42f
private const val PIP_MAX_RATIO = 2.39f

/**
 * 系统风格的整屏播放器：Media3 的 `PlayerView`（拖动、音轨字幕、播放速度菜单都是它的），播放器在
 * [VideoPlaybackService] 里，这里用 `MediaController` 连过去。按 Home 时在放就进画中画；关掉界面（返回、关小窗）
 * 就结束这一次，报停在哪；放完报看完并关掉。
 */
class VideoPlayerActivity : ComponentActivity() {
    private var future: ListenableFuture<MediaController>? = null
    private var controller: MediaController? = null
    private var run: VideoRun? = null
    private lateinit var view: PlayerView

    private val listener = object : Player.Listener {
        override fun onIsPlayingChanged(isPlaying: Boolean) {
            run?.playing(isPlaying)
            updatePictureInPicture()
        }

        override fun onPositionDiscontinuity(
            oldPosition: Player.PositionInfo,
            newPosition: Player.PositionInfo,
            reason: Int,
        ) {
            if (reason == Player.DISCONTINUITY_REASON_SEEK) run?.report(force = true)
        }

        override fun onPlaybackStateChanged(playbackState: Int) {
            if (playbackState == Player.STATE_ENDED) {
                run?.close(finished = true)
                finish()
            }
        }

        override fun onPlayerError(error: PlaybackException) {
            run?.fail(error.message ?: error.errorCodeName)
            finish()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        view = PlayerView(this).apply {
            setBackgroundColor(Color.BLACK)
            setShowNextButton(false)
            setShowPreviousButton(false)
            setShowSubtitleButton(true)
            keepScreenOn = true
        }
        setContentView(view)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, view).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        val token = SessionToken(this, ComponentName(this, VideoPlaybackService::class.java))
        val pending = MediaController.Builder(this, token).buildAsync()
        future = pending
        pending.addListener({
            val connected = runCatching { pending.get() }.getOrElse { failure ->
                VideoPlayback.current?.fail(failure.message ?: failure.toString())
                finish()
                return@addListener
            }
            controller = connected
            connected.addListener(listener)
            view.player = connected
            load()
        }, ContextCompat.getMainExecutor(this))
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        load()
    }

    /** 放 [VideoPlayback] 里当前这一次；已经在放它就不动。 */
    private fun load() {
        val player = controller ?: return
        val next = VideoPlayback.current
        if (next == null || next.closed) {
            finish()
        } else if (next !== run) {
            run = next
            next.position = { player.currentPosition }
            val request = next.request
            val item = MediaItem.Builder()
                .setUri(request.url)
                .setMediaMetadata(MediaMetadata.Builder().setTitle(request.title).build())
                .apply { if (request.transcoded) setMimeType(MimeTypes.APPLICATION_M3U8) }
                .build()
            player.setMediaItem(item, request.positionMs.toLong())
            player.prepare()
            player.play()
        }
    }

    /** 在放时按 Home 自动进画中画（Android 12 起由系统做，之前在 [onUserLeaveHint] 里手动进）。 */
    private fun updatePictureInPicture() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            setPictureInPictureParams(pictureInPictureParams(autoEnter = controller?.isPlaying == true))
        }
    }

    private fun pictureInPictureParams(autoEnter: Boolean): PictureInPictureParams {
        val size = controller?.videoSize
        val builder = PictureInPictureParams.Builder()
        if (size != null && size.width > 0 && size.height > 0) {
            val ratio = (size.width.toFloat() / size.height).coerceIn(PIP_MIN_RATIO, PIP_MAX_RATIO)
            builder.setAspectRatio(Rational((ratio * size.height).toInt(), size.height))
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) builder.setAutoEnterEnabled(autoEnter)
        return builder.build()
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S && controller?.isPlaying == true) {
            enterPictureInPictureMode(pictureInPictureParams(autoEnter = false))
        }
    }

    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        view.useController = !isInPictureInPictureMode
        // 小窗被关掉（没回到整屏）：界面已经停下，这次播放结束。
        if (!isInPictureInPictureMode && lifecycle.currentState == Lifecycle.State.CREATED) finish()
    }

    override fun onDestroy() {
        if (isFinishing) {
            run?.close(finished = false)
            controller?.run {
                stop()
                clearMediaItems()
            }
        }
        controller?.removeListener(listener)
        view.player = null
        future?.let(MediaController::releaseFuture)
        super.onDestroy()
    }
}
