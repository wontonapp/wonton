package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import app.wonton.client.ClientCore
import app.wonton.protocol.TimelineTarget

/** 编辑态由核心给（ADR-0109 第 8 节）：进来时把原正文放进输入框，结束后换回核心恢复的草稿。 */
@Composable
internal fun RoomDraftEffects(
    timeline: TimelineState,
    draft: RoomDraft,
    core: ClientCore,
    target: TimelineTarget,
) {
    LaunchedEffect(timeline.editing) {
        val edit = timeline.editing
        if (edit != null && draft.editing?.eventId != edit.eventId) {
            draft.beginEdit(edit, runCatching { core.draft(target) }.getOrNull()?.body)
        } else if (edit == null && draft.editing != null) {
            draft.endEdit(runCatching { core.draft(target) }.getOrNull())
        }
    }
    RoomPanelEffects(timeline, draft)
}

@Composable
internal fun RoomPanelEffects(timeline: TimelineState, draft: RoomDraft) {
    val panel = LocalChatPanel.current
    val panelPages = timeline.panel
    val panelSelected = timeline.panelSelected
    SideEffect {
        panel?.feed(panelPages, panelSelected)
    }
    DisposableEffect(panel, draft) {
        panel?.draftSink = { text -> draft.change(if (draft.text.isBlank()) text else "${draft.text}\n$text") }
        onDispose { panel?.draftSink = null }
    }
}
