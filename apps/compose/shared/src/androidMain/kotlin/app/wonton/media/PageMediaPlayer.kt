@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.media

import android.os.Looper
import androidx.media3.common.C
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import app.wonton.client.PageMediaAction
import app.wonton.client.PageMediaPlayback
import app.wonton.client.PageMediaState
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture

internal class PageMediaPlayer(
    private val command: (PageMediaAction, ULong?) -> Unit,
) : SimpleBasePlayer(Looper.getMainLooper()) {
    private var snapshot: PageMediaState? = null

    private var position: PositionSupplier = PositionSupplier.ZERO

    fun update(value: PageMediaState) {
        snapshot = value
        val start = value.positionMs?.toLong() ?: 0L
        val limit = value.durationMs?.toLong()
        position = if (value.playback == PageMediaPlayback.PLAYING) {
            val moving = PositionSupplier.getExtrapolating(start, value.rate.toFloat())
            // 页面在下一条消息到来前不再报位置，系统会一直外推；到时长就停住，别越过进度条。
            if (limit == null) moving else PositionSupplier { moving.get().coerceAtMost(limit) }
        } else {
            PositionSupplier.getConstant(start)
        }
        invalidateState()
    }

    override fun getState(): State {
        val value = snapshot ?: return State.Builder().build()
        val metadata = MediaMetadata.Builder()
            .setTitle(value.title)
            .setArtist(value.artist)
            .setAlbumTitle(value.album)
            .setArtworkData(value.artwork, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
            .build()
        val item = MediaItemData.Builder("page-media")
            .setMediaMetadata(metadata)
            .setDurationUs(value.durationMs?.toLong()?.times(MICROS_PER_MILLI) ?: C.TIME_UNSET)
            .setIsSeekable(PageMediaAction.SEEK_TO in value.actions)
            .build()
        return State.Builder()
            .setAvailableCommands(availableCommands(value))
            .setPlaylist(listOf(item))
            .setCurrentMediaItemIndex(0)
            .setPlayWhenReady(value.playback == PageMediaPlayback.PLAYING,
                Player.PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setPlaybackState(Player.STATE_READY)
            .setPlaybackParameters(PlaybackParameters(value.rate.toFloat()))
            .setContentPositionMs(position)
            .build()
    }

    private fun availableCommands(value: PageMediaState): Player.Commands {
        val commands = Player.Commands.Builder().addAll(
            Player.COMMAND_GET_CURRENT_MEDIA_ITEM, Player.COMMAND_GET_METADATA,
            Player.COMMAND_GET_TIMELINE, Player.COMMAND_RELEASE,
        )
        value.actions.forEach { action ->
            when (action) {
                PageMediaAction.PLAY, PageMediaAction.PAUSE -> commands.add(Player.COMMAND_PLAY_PAUSE)
                PageMediaAction.STOP -> commands.add(Player.COMMAND_STOP)
                PageMediaAction.PREVIOUS_TRACK -> commands.addAll(
                    Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                PageMediaAction.NEXT_TRACK -> commands.addAll(
                    Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
                PageMediaAction.SEEK_BACKWARD -> commands.add(Player.COMMAND_SEEK_BACK)
                PageMediaAction.SEEK_FORWARD -> commands.add(Player.COMMAND_SEEK_FORWARD)
                PageMediaAction.SEEK_TO -> commands.add(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM)
                else -> Unit
            }
        }
        return commands.build()
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        command(if (playWhenReady) PageMediaAction.PLAY else PageMediaAction.PAUSE, null)
        return Futures.immediateVoidFuture()
    }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        val action = when (seekCommand) {
            Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> PageMediaAction.NEXT_TRACK
            Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM ->
                PageMediaAction.PREVIOUS_TRACK
            Player.COMMAND_SEEK_BACK -> PageMediaAction.SEEK_BACKWARD
            Player.COMMAND_SEEK_FORWARD -> PageMediaAction.SEEK_FORWARD
            else -> PageMediaAction.SEEK_TO
        }
        val seek = when (action) {
            PageMediaAction.SEEK_TO -> positionMs.coerceAtLeast(0).toULong()
            PageMediaAction.SEEK_BACKWARD -> seekBackIncrement.toULong()
            PageMediaAction.SEEK_FORWARD -> seekForwardIncrement.toULong()
            else -> null
        }
        command(action, seek)
        return Futures.immediateVoidFuture()
    }

    override fun handleStop(): ListenableFuture<*> {
        command(PageMediaAction.STOP, null)
        return Futures.immediateVoidFuture()
    }

    override fun handleRelease(): ListenableFuture<*> = Futures.immediateVoidFuture()

    private companion object {
        const val MICROS_PER_MILLI = 1000L
    }
}
