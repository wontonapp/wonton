package app.wonton.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BackPagination
import app.wonton.protocol.CoreException
import app.wonton.protocol.TimelineDiff
import app.wonton.protocol.TimelineItem
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch

private class TimelineState {
    val items = mutableStateListOf<TimelineItem>()
    var backPagination by mutableStateOf(BackPagination.IDLE)

    /** 收到第一批（全部条目）之后才能翻页：核心要先打开这个房间的时间线。 */
    var loaded by mutableStateOf(false)
    var failure by mutableStateOf<String?>(null)

    fun apply(diff: TimelineDiff) {
        when (diff) {
            is TimelineDiff.Append -> items.addAll(diff.items)
            TimelineDiff.Clear -> items.clear()
            is TimelineDiff.PushFront -> items.add(0, diff.item)
            is TimelineDiff.PushBack -> items.add(diff.item)
            TimelineDiff.PopFront -> items.removeAt(0)
            TimelineDiff.PopBack -> items.removeAt(items.lastIndex)
            is TimelineDiff.Insert -> items.add(diff.index.toInt(), diff.item)
            is TimelineDiff.Set -> items[diff.index.toInt()] = diff.item
            is TimelineDiff.Remove -> items.removeAt(diff.index.toInt())
            is TimelineDiff.Truncate -> items.removeRange(diff.length.toInt(), items.size)
            is TimelineDiff.Reset -> {
                items.clear()
                items.addAll(diff.items)
            }
        }
    }
}

@Composable
fun RoomRoute(
    core: ClientCore,
    roomId: String,
    title: String,
    isDirect: Boolean,
    onAction: (Action) -> Unit,
    onReject: (String) -> Unit,
    onClose: () -> Unit,
) {
    val timeline = remember(roomId) { TimelineState() }
    LaunchedEffect(core, roomId) {
        try {
            core.timelineUpdates(roomId).collect { update ->
                update.diffs.forEach(timeline::apply)
                timeline.backPagination = update.backPagination
                timeline.loaded = true
            }
        } catch (failure: CoreException) {
            timeline.failure = failure.toString()
        }
    }
    val newest = timeline.items.lastOrNull()?.id
    LaunchedEffect(newest) {
        if (newest != null) onAction(Action.MarkRoomRead(roomId))
    }
    RoomScreen(core, roomId, title, isDirect, timeline, onAction, onReject, onClose)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RoomScreen(
    core: ClientCore,
    roomId: String,
    title: String,
    isDirect: Boolean,
    timeline: TimelineState,
    onAction: (Action) -> Unit,
    onReject: (String) -> Unit,
    onClose: () -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1) },
                navigationIcon = { TextButton(onClick = onClose) { Text("返回") } },
            )
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize().imePadding()) {
            val failure = timeline.failure
            if (failure != null) {
                Box(Modifier.weight(1f).fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
                    Text("打不开这个聊天：$failure")
                }
            } else {
                Messages(core, roomId, timeline, showsSender = !isDirect, onAction, Modifier.weight(1f))
            }
            Composer(roomId, onAction, onReject)
        }
    }
}

@Composable
private fun Messages(
    core: ClientCore,
    roomId: String,
    timeline: TimelineState,
    showsSender: Boolean,
    onAction: (Action) -> Unit,
    modifier: Modifier,
) {
    val listState = rememberLazyListState()
    // 倒着排：第 0 项是最新的一条，贴着底部。
    val newestFirst = timeline.items.asReversed()
    // 新消息插在第 0 项时，列表保持原来看到的那条不动，新消息落在屏幕下面；原本就在底部时滚过去。
    val newest = timeline.items.lastOrNull()?.id
    LaunchedEffect(newest) {
        if (newest != null && listState.firstVisibleItemIndex <= 1) listState.animateScrollToItem(0)
    }
    // 滚到最上面（最早的一条露出来）时加载更早的消息；加载完条目变多，还在顶上就再加载一页。
    LaunchedEffect(listState, roomId) {
        snapshotFlow {
            val last = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index
            timeline.loaded && last != null && last >= listState.layoutInfo.totalItemsCount - 1 &&
                timeline.backPagination == BackPagination.IDLE
        }
            .distinctUntilChanged()
            .filter { it }
            .collect { onAction(Action.PaginateBackwards(roomId)) }
    }
    LazyColumn(
        modifier.fillMaxWidth(),
        state = listState,
        reverseLayout = true,
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(newestFirst, key = { it.id }) { item -> TimelineItemView(item, core, showsSender) }
        if (timeline.backPagination != BackPagination.REACHED_START) {
            item(key = "paginating") {
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            }
        }
    }
}

@Composable
private fun Composer(roomId: String, onAction: (Action) -> Unit, onReject: (String) -> Unit) {
    var draft by rememberSaveable(roomId) { mutableStateOf("") }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val pickPhoto = rememberLauncherForActivityResult(PickVisualMedia()) { uri ->
        if (uri != null) {
            scope.launch {
                PhotoUpload.make(context, uri).fold(
                    onSuccess = { upload -> onAction(Action.SendImage(roomId, upload)) },
                    onFailure = { failure -> onReject(failure.message ?: failure.toString()) },
                )
            }
        }
    }
    Row(
        Modifier.fillMaxWidth().padding(8.dp),
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        TextButton(onClick = { pickPhoto.launch(PickVisualMediaRequest(PickVisualMedia.ImageOnly)) }) {
            Text("图片")
        }
        OutlinedTextField(
            value = draft,
            onValueChange = { draft = it },
            modifier = Modifier.weight(1f),
            placeholder = { Text("消息") },
            maxLines = 5,
        )
        TextButton(
            onClick = {
                onAction(Action.SendText(roomId, draft))
                draft = ""
            },
            enabled = draft.isNotBlank(),
        ) { Text("发送") }
    }
}
