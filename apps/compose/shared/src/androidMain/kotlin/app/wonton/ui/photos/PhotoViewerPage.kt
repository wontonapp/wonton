package app.wonton.ui.photos

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.MediaKind
import app.wonton.protocol.PhotoRendition
import app.wonton.protocol.PhotoTile
import app.wonton.ui.Problem
import app.wonton.ui.ProblemDetail
import app.wonton.ui.R
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.PlayFill
import app.wonton.ui.icons.WontonIcon
import coil3.compose.AsyncImagePainter

private val CLOSE_DRAG = 120.dp
private val PLAY_BUTTON = 72.dp
private val NOTICE_WIDTH = 360.dp

/** 大图页告诉外层的事：点了一下（收放顶栏）、要关、放大没放大、下滑了多远。 */
internal class ViewerPageEvents(
    val onTap: () -> Unit,
    val onClose: () -> Unit,
    val onZoomed: (Boolean) -> Unit,
    val onDrag: (Float) -> Unit,
)

/**
 * 大图页的一页：预览档的图，双击和两指缩放、放大后拖动、没放大时下滑关闭。视频点中间的播放钮就地放；
 * 实况照片按住就放动态那一段，松手停。[active] 是不是正对着的这一页：翻走就停播、还原缩放。
 */
@Composable
internal fun PhotoViewerPage(state: PhotosState, tile: PhotoTile, active: Boolean, events: ViewerPageEvents) {
    val scope = rememberCoroutineScope()
    val zoom = remember(tile.id) { PhotoZoom(scope) }
    val latest by rememberUpdatedState(events)
    val closeDistance = with(LocalDensity.current) { CLOSE_DRAG.toPx() }
    var playing by remember(tile.id) { mutableStateOf<PhotoRendition?>(null) }
    var problem by remember(tile.id) { mutableStateOf<Problem?>(null) }
    var retry by remember(tile.id) { mutableIntStateOf(0) }
    LaunchedEffect(active) {
        if (!active) {
            playing = null
            zoom.reset()
        }
    }
    if (active) {
        LaunchedEffect(zoom.zoomed) { latest.onZoomed(zoom.zoomed) }
        LaunchedEffect(zoom.dismiss) { latest.onDrag(zoom.dismiss) }
    }
    Box(
        Modifier.fillMaxSize()
            .onSizeChanged { zoom.viewport = it }
            .pointerInput(tile.id, tile.live) {
                detectTapGestures(
                    onTap = { latest.onTap() },
                    onDoubleTap = { zoom.toggle(it) },
                    onLongPress = { if (tile.live) playing = PhotoRendition.LIVE },
                    onPress = {
                        tryAwaitRelease()
                        if (playing == PhotoRendition.LIVE) playing = null
                    },
                )
            }
            .pointerInput(zoom) { photoZoomGestures(zoom, closeDistance) { latest.onClose() } },
    ) {
        Box(
            Modifier.fillMaxSize().graphicsLayer {
                scaleX = zoom.scale
                scaleY = zoom.scale
                translationX = zoom.offset.x
                translationY = zoom.offset.y + zoom.dismiss
            },
        ) {
            PhotoImage(
                state.ref(tile.id, PhotoRendition.PREVIEW), tile.thumbhash, tileTitle(tile),
                Modifier.fillMaxSize(), ContentScale.Fit, retry,
            ) { loaded ->
                if (loaded is AsyncImagePainter.State.Error) problem = loaded.result.throwable.photoProblem()
                if (loaded is AsyncImagePainter.State.Success) problem = null
            }
            playing?.let { rendition ->
                PhotoVideo(
                    state, tile.id, rendition, controls = rendition == PhotoRendition.PLAYBACK,
                    onEnded = { playing = null },
                ) { failure ->
                    playing = null
                    problem = failure
                }
            }
        }
        if (playing == null && problem == null) PageControls(tile) { playing = it }
        problem?.let { failure ->
            PageNotice(failure) {
                problem = null
                retry += 1
            }
        }
    }
}

@Composable
private fun tileTitle(tile: PhotoTile): String = stringResource(
    if (tile.kind == MediaKind.Video) R.string.photo_library_video else R.string.photo_library_photo,
)

/** 压在图上的播放入口：视频正中一个播放钮；实况照片底下一颗“实况”，点它或按住图都放。 */
@Composable
private fun BoxScope.PageControls(tile: PhotoTile, onPlay: (PhotoRendition) -> Unit) {
    if (tile.kind == MediaKind.Video) {
        val label = stringResource(R.string.photo_library_play)
        Box(
            Modifier.align(Alignment.Center).size(PLAY_BUTTON).background(PhotoScrim, CircleShape)
                .clickable(role = Role.Button) { onPlay(PhotoRendition.PLAYBACK) }
                .semantics { contentDescription = label },
            contentAlignment = Alignment.Center,
        ) {
            // 装饰：外层按钮已经带了名字。
            Icon(WontonIcon.PlayFill, null, Modifier.size(WontonTheme.size.icon), tint = Color.White)
        }
    } else if (tile.live) {
        Row(
            Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = WontonTheme.spacing.s24)
                .background(PhotoScrim, CircleShape)
                .clickable(role = Role.Button) { onPlay(PhotoRendition.LIVE) }
                .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            LiveBadge(WontonTheme.size.iconTiny)
            Text(
                stringResource(R.string.photo_library_live),
                style = WontonTheme.typography.supporting,
                color = Color.White,
            )
        }
    }
}

/** 这一张取不到、放不了：压在占位图上的一块说明，带重试，核心写的原文收在“详情”。 */
@Composable
private fun BoxScope.PageNotice(problem: Problem, onRetry: () -> Unit) {
    val colors = WontonTheme.colors
    Column(
        Modifier.align(Alignment.Center).padding(WontonTheme.spacing.s24).widthIn(max = NOTICE_WIDTH).fillMaxWidth()
            .background(colors.surface.toColor(), RoundedCornerShape(WontonTheme.radii.card))
            .padding(WontonTheme.spacing.s16),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(problem.titleText, style = WontonTheme.typography.itemTitle, color = colors.textPrimary.toColor())
        problem.nextStepText?.let {
            Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        }
        WontonSmallButton(stringResource(R.string.common_retry), onRetry)
        problem.detail?.let { ProblemDetail(it) }
    }
}
