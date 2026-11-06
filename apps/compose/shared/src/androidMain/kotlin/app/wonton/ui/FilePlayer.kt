package app.wonton.ui

import android.net.Uri
import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.FileInfo
import app.wonton.protocol.FileProblem
import app.wonton.ui.design.WontonFontScale

/**
 * 边下边播，用系统的 `VideoView` 放核心给的回环地址：支持 Range，音频也能放，不多带一个播放器库。放不了时问核心为什么，说清原因，
 * 并给一个下载后用其他 App 打开的按钮。
 */
@Composable
fun FilePlayer(info: FileInfo, core: ClientCore, onClose: () -> Unit, onOpenElsewhere: () -> Unit) {
    var url by remember { mutableStateOf<String?>(null) }
    var failed by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<FileProblem?>(null) }
    LaunchedEffect(info.id) {
        try {
            url = core.fileStreamUrl(info.id)
        } catch (_: CoreException) {
            failed = true
        }
    }
    LaunchedEffect(failed) {
        if (failed) problem = core.filePlaybackProblem(info.id)
    }
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        WontonFontScale {
            Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
                val address = url
                val shown = problem
                when {
                    shown != null -> PlayerProblem(shown, onOpenElsewhere)
                    failed || address == null -> CircularProgressIndicator()
                    else -> PlayerSurface(address, onFailed = { failed = true })
                }
                TextButton(onClick = onClose, Modifier.align(Alignment.TopEnd).padding(8.dp)) {
                    Text(stringResource(R.string.common_done), color = Color.White)
                }
            }
        }
    }
}

@Composable
private fun PlayerProblem(problem: FileProblem, onOpenElsewhere: () -> Unit) {
    Column(
        Modifier.padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(problem.message, color = Color.White, textAlign = TextAlign.Center)
        TextButton(onClick = onOpenElsewhere) { Text(stringResource(R.string.file_download_and_open)) }
    }
}

@Composable
private fun PlayerSurface(url: String, onFailed: () -> Unit) {
    // 切后台时 surface 被销毁，VideoView 释放播放器，不记位置，回来重新准备：自己记下放到哪了、放没放着。重新准备好之前再切走，
    // VideoView 的位置是 0，不能记。
    var position by remember { mutableIntStateOf(0) }
    var playsOnReturn by remember { mutableStateOf(true) }
    var prepared by remember { mutableStateOf(false) }
    var player by remember { mutableStateOf<VideoView?>(null) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, player) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_PAUSE && prepared) {
                player?.let {
                    position = it.currentPosition
                    playsOnReturn = it.isPlaying
                }
                prepared = false
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    AndroidView(
        factory = { context ->
            VideoView(context).apply {
                player = this
                setMediaController(MediaController(context).also { it.setAnchorView(this) })
                setOnPreparedListener {
                    prepared = true
                    seekTo(position)
                    if (playsOnReturn) start()
                }
                setOnErrorListener { _, _, _ ->
                    onFailed()
                    true
                }
                setVideoURI(Uri.parse(url))
            }
        },
        onRelease = VideoView::stopPlayback,
    )
}
