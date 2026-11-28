@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
@file:Suppress("TooManyFunctions") // 播放器的状态和通往 ExoPlayer、核心的几条路在一处；拆开就得互相露出私有状态。

package app.wonton.media

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import app.wonton.client.ClientCore
import app.wonton.client.PlayerCommand
import app.wonton.client.PlayerEvent
import app.wonton.client.PlayerPhase
import app.wonton.client.PlayerSupport
import app.wonton.protocol.Action
import app.wonton.protocol.ControlOp
import app.wonton.protocol.MediaAction
import app.wonton.protocol.MediaException
import app.wonton.protocol.MediaFailure
import app.wonton.protocol.MediaFailureCode
import app.wonton.protocol.MediaItemRef
import app.wonton.protocol.PlaybackSession
import app.wonton.protocol.PlaybackViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** 播放中每隔这么久取一次位置：进度条跟着走，也报给核心（多久真的发一次由核心定）。 */
private const val TICK_MS = 1_000L

/** 放过这么久按“上一首”是回到开头，和系统播放器一样。 */
private const val RESTART_AFTER_MS = 3_000L

/** 通知栏、锁屏和播放页的封面取这么宽。 */
private const val ARTWORK_PIXELS = 720u

/** ExoPlayer 的错误码按千分组，2xxx 是读数据（连不上回环、读断了）。 */
private val IO_ERRORS = 2000..2999

/**
 * 原生音乐的播放器（ADR-0113 第 4 节）：会话、队列、换曲都在核心，这里照播放视图出声，播放器发生的事报回核心
 * （报的节奏、放完换下一首还是停由核心定）。整个 App 只有一个，离开音乐页也接着放；通知栏、锁屏和耳机的按钮经
 * [MusicPlaybackService] 回到这里。
 */
internal object MusicPlayback {
    /** 这台设备当前的播放；停了的会话也留着，显示停在哪。 */
    var view by mutableStateOf<PlaybackViewModel?>(null)
        private set
    var isPlaying by mutableStateOf(false)
        private set
    var positionMs by mutableStateOf(0L)
        private set
    var artwork by mutableStateOf<Bitmap?>(null)
        private set

    /** 起播、出声没成：弹一句。 */
    var failure by mutableStateOf<MediaFailure?>(null)

    private var core: ClientCore? = null
    private var context: Context? = null
    private var exo: ExoPlayer? = null
    private var following: Job? = null
    private var ticking: Job? = null
    private val scope = MainScope()

    /** 播放器里装着的是哪条会话的哪一项：两样都没变就不重开流。 */
    private var loaded: Pair<String, String>? = null

    /** 照核心的状态动播放器时，播放器回报的变化不再当成用户的操作报回去。 */
    private var applying = false

    /** 有会话、没停：迷你播放条和播放页才出现。 */
    val isActive: Boolean get() = view?.session?.stopReason == null && view != null

    val durationMs: Long
        get() {
            val planned = view?.plan?.durationMs?.toLong() ?: 0L
            return if (planned > 0) planned else view?.session?.item?.durationMs?.toLong() ?: 0L
        }

    /** 点了音乐页上的一首歌：[start] 是从这首起放，没成的原因弹出来。 */
    fun play(core: ClientCore, context: Context, start: suspend () -> Unit) {
        attach(core, context.applicationContext)
        scope.launch {
            try {
                start()
            } catch (error: MediaException) {
                failure = error.reason
            }
        }
    }

    fun togglePlayPause() {
        val player = exo ?: return
        if (player.playWhenReady) player.pause() else player.play()
    }

    fun next() = control(ControlOp.Next)

    fun previous() {
        if (currentMs > RESTART_AFTER_MS) seek(0) else control(ControlOp.Prev)
    }

    fun seek(positionMs: Long) {
        exo?.seekTo(positionMs)
        this.positionMs = positionMs
    }

    /** 给通知栏和锁屏用的播放器：上一首、下一首交回这里，别的照 ExoPlayer。 */
    fun sessionPlayer(): Player? = exo?.let(::MusicSessionPlayer)

    /** 换了服务器（核心换了一个）：跟着新的核心走，旧会话的声音停掉。 */
    private fun attach(client: ClientCore, app: Context) {
        if (core === client) return
        following?.cancel()
        stopHere()
        core = client
        context = app
        // 只报音频：视频的播放器另报，后报的覆盖先报的，所以把 ExoPlayer 自己能解的都写上。
        runCatching {
            client.reportPlayerSupport(
                PlayerSupport(
                    mimeTypes = listOf(
                        "audio/mpeg", "audio/aac", "audio/mp4", "audio/x-m4a", "audio/flac", "audio/x-flac",
                        "audio/wav", "audio/x-wav", "audio/ogg", "audio/opus", "audio/webm",
                    ),
                    containers = listOf("mp3", "aac", "m4a", "mp4", "flac", "wav", "ogg", "oga", "opus", "webm"),
                    maxBitrate = 0u,
                    hdr = false,
                ),
            )
        }
        following = scope.launch {
            client.subscribePlayback().use { subscription ->
                while (true) apply(subscription.next() ?: break)
            }
        }
    }

    private fun player(): ExoPlayer = exo ?: ExoPlayer.Builder(checkNotNull(context))
        .setAudioAttributes(
            AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(),
            true,
        )
        .setHandleAudioBecomingNoisy(true)
        .setWakeMode(C.WAKE_MODE_NETWORK)
        .build()
        .also {
            it.addListener(Listening)
            exo = it
        }

    private val currentMs: Long get() = exo?.currentPosition?.coerceAtLeast(0) ?: 0L

    private fun report(event: PlayerEvent) {
        val command = core?.reportPlayer(event) ?: return
        if (command is PlayerCommand.SeekTo) {
            quietly {
                exo?.seekTo(command.positionMs.toLong())
                exo?.play()
            }
        }
    }

    private fun control(ctl: ControlOp) {
        val session = view?.session?.takeIf { it.stopReason == null } ?: return
        core?.let { client ->
            runCatching { client.dispatch(Action.Media(MediaAction.Control(session.id, ctl))) }.onFailure {
                failure = MediaFailure(MediaFailureCode.TransportUnavailable, it.message.orEmpty(), null)
            }
        }
    }

    private inline fun quietly(change: () -> Unit) {
        applying = true
        try {
            change()
        } finally {
            applying = false
        }
    }

    /** 播放视图来了：会话停了就停声并补报最后的位置，还在放就跟着它。 */
    private suspend fun apply(next: PlaybackViewModel) {
        view = next
        next.failure?.let { failure = it }
        val session = next.session
        when {
            session.stopReason != null -> stopHere()
            next.plan != null -> follow(session)
        }
    }

    /** 会话在放：换了会话或条目就重开流，再照核心的状态放或停。 */
    private suspend fun follow(session: PlaybackSession) {
        val wanted = session.id to session.item.id
        val ready = loaded == wanted || load(session, wanted)
        val player = exo
        if (!ready || player == null) return
        when (core?.playerState()?.phase) {
            PlayerPhase.PLAYING -> quietly { player.play() }
            PlayerPhase.PAUSED -> quietly { player.pause() }
            else -> Unit
        }
    }

    private suspend fun load(session: PlaybackSession, wanted: Pair<String, String>): Boolean {
        val client = core ?: return false
        val address = try {
            client.playbackStreamUrl(session.id)
        } catch (error: MediaException) {
            failure = error.reason
            null
        }
        if (address != null) {
            val item = MediaItem.Builder()
                .setUri(address)
                .setMediaId(session.item.id)
                .setMediaMetadata(metadata(session.item, null))
                .build()
            val player = player()
            quietly {
                player.setMediaItem(item, session.positionMs.toLong())
                player.prepare()
            }
            loaded = wanted
            positionMs = session.positionMs.toLong()
            artwork = null
            loadArtwork(client, session.item)
        }
        return address != null
    }

    private fun loadArtwork(client: ClientCore, item: MediaItemRef) {
        scope.launch {
            val bytes = client.mediaArtwork(item, ARTWORK_PIXELS) ?: return@launch
            if (loaded?.second != item.id) return@launch
            artwork = withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size) }
            val player = exo ?: return@launch
            val current = player.currentMediaItem?.takeIf { it.mediaId == item.id } ?: return@launch
            player.replaceMediaItem(0, current.buildUpon().setMediaMetadata(metadata(item, bytes)).build())
        }
    }

    /** 会话在别处停了（另一台设备接着放、Agent 停了）或者这里放完了：停声，最后的位置报一次。 */
    private fun stopHere() {
        val player = exo ?: return
        if (loaded == null) return
        quietly { player.pause() }
        report(PlayerEvent.Stopped(currentMs.toULong()))
        quietly {
            player.stop()
            player.clearMediaItems()
        }
        loaded = null
        artwork = null
        context?.let { MusicPlaybackService.stop(it) }
    }

    private fun metadata(item: MediaItemRef, artwork: ByteArray?): MediaMetadata = MediaMetadata.Builder()
        .setTitle(item.title)
        .setArtist(item.artist)
        .setAlbumTitle(item.album)
        .setArtworkData(artwork, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
        .build()

    private fun startTicking() {
        if (ticking?.isActive == true) return
        ticking = scope.launch {
            while (isActive) {
                positionMs = currentMs
                report(PlayerEvent.Playing(positionMs.toULong()))
                delay(TICK_MS)
            }
        }
    }

    /** ExoPlayer 发生的事：出声、暂停（用户、通知栏、耳机拔了、来电）、拖动、放完、放不下去。 */
    private object Listening : Player.Listener {
        override fun onIsPlayingChanged(playing: Boolean) {
            isPlaying = playing
            if (playing) {
                startTicking()
                context?.let { MusicPlaybackService.start(it) }
            } else {
                ticking?.cancel()
                positionMs = currentMs
            }
        }

        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            if (applying || reason == Player.PLAY_WHEN_READY_CHANGE_REASON_END_OF_MEDIA_ITEM) return
            if (playWhenReady) {
                control(ControlOp.Resume)
            } else {
                report(PlayerEvent.Paused(currentMs.toULong()))
                control(ControlOp.Pause)
            }
        }

        override fun onPositionDiscontinuity(
            oldPosition: Player.PositionInfo,
            newPosition: Player.PositionInfo,
            reason: Int,
        ) {
            if (applying || reason != Player.DISCONTINUITY_REASON_SEEK) return
            report(PlayerEvent.Seeked(newPosition.positionMs.coerceAtLeast(0).toULong()))
        }

        override fun onPlaybackStateChanged(state: Int) {
            if (state == Player.STATE_ENDED) report(PlayerEvent.Finished(durationMs.toULong()))
        }

        override fun onPlayerError(error: PlaybackException) {
            // 读不到数据是回环或隧道断了；读到了还放不了多半是格式。
            val code = if (error.errorCode in IO_ERRORS) {
                MediaFailureCode.TransportUnavailable
            } else {
                MediaFailureCode.UnsupportedByDevice
            }
            report(PlayerEvent.Failed(currentMs.toULong(), code, "${error.errorCodeName}: ${error.message}"))
        }
    }
}

/** 导出的媒体操作只有一种失败，带着核心归好类的原因。 */
internal val MediaException.reason: MediaFailure
    get() = when (this) {
        is MediaException.Failed -> failure
    }
