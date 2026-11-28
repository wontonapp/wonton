package app.wonton.ui.photos

import android.net.Uri
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.protocol.MediaException
import app.wonton.protocol.PhotoRendition
import app.wonton.ui.Problem
import app.wonton.ui.R

/**
 * 在大图页里直接放一段：视频走 `playback` 档，实况照片的动态部分走 `live` 档，都是核心给的本机回环地址（支持 Range），
 * 交给系统的 `VideoView`。[controls] 给视频出系统的播放控制条；实况不出，放完叫 [onEnded]。
 * 地址要不到、放不了时把原因交给 [onProblem]。
 */
@Composable
internal fun PhotoVideo(
    state: PhotosState,
    id: String,
    rendition: PhotoRendition,
    controls: Boolean,
    modifier: Modifier = Modifier,
    onEnded: () -> Unit = {},
    onProblem: (Problem) -> Unit,
) {
    val url by produceState<String?>(null, id, rendition, state.source) {
        value = try {
            state.core.photoUrl(id, rendition)
        } catch (failure: MediaException) {
            onProblem(failure.photoProblem())
            null
        }
    }
    Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val address = url
        if (address == null) {
            CircularProgressIndicator(color = Color.White)
        } else {
            VideoSurface(address, controls, onEnded) {
                onProblem(
                    Problem(
                        titleRes = R.string.photo_library_video_failed_title,
                        nextStepRes = R.string.photo_library_try_later,
                    ),
                )
            }
        }
    }
}

@Composable
private fun VideoSurface(url: String, controls: Boolean, onEnded: () -> Unit, onFailed: () -> Unit) {
    // 切后台时 surface 被销毁，VideoView 不记位置：自己记下放到哪了，回来接着放。
    var position by remember { mutableIntStateOf(0) }
    var player by remember { mutableStateOf<VideoView?>(null) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, player) {
        val observer = LifecycleEventObserver { _, event ->
            val playing = player?.takeIf { it.isPlaying }
            if (event == Lifecycle.Event.ON_PAUSE && playing != null) position = playing.currentPosition
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    AndroidView(
        factory = { context ->
            val video = VideoView(context).apply {
                if (controls) setMediaController(MediaController(context).also { it.setAnchorView(this) })
                setOnPreparedListener {
                    seekTo(position)
                    start()
                }
                setOnCompletionListener { onEnded() }
                setOnErrorListener { _, _, _ ->
                    onFailed()
                    true
                }
                setVideoURI(Uri.parse(url))
            }
            player = video
            // VideoView 按视频的宽高比收自己的尺寸；外面套一层把它摆在正中。
            FrameLayout(context).apply {
                val fill = FrameLayout.LayoutParams.MATCH_PARENT
                addView(video, FrameLayout.LayoutParams(fill, fill, Gravity.CENTER))
            }
        },
        modifier = Modifier.fillMaxSize(),
        onRelease = { player?.stopPlayback() },
    )
}
