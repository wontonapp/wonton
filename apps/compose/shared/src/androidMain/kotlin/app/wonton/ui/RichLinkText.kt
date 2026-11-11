package app.wonton.ui

import android.content.ClipData
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import app.wonton.protocol.RichLink
import app.wonton.protocol.RichSpan
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.icons.Copy
import app.wonton.ui.icons.Globe
import app.wonton.ui.icons.Share
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** Link annotations retain accessibility; link gestures are consumed before the bubble sees them. */
@Composable
internal fun RichLinkText(
    text: AnnotatedString,
    spans: List<RichSpan>,
    modifier: Modifier = Modifier,
    style: TextStyle = LocalTextStyle.current,
) {
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    val opener = LocalLinkOpener.current
    var selected by remember { mutableStateOf<RichLink?>(null) }
    val haptics = LocalHapticFeedback.current
    Text(
        text,
        modifier.pointerInput(text, spans, opener) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                val link = layout?.let { linkAt(it, spans, down.position) } ?: return@awaitEachGesture
                down.consume()
                val tapped = withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                    var result: Boolean? = null
                    while (result == null) {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        if (event.changes.any {
                            (it.position - down.position).getDistance() > viewConfiguration.touchSlop
                        }) result = false
                        if (event.changes.none { it.pressed } && result == null) result = true
                        if (result == true) event.changes.forEach { it.consume() }
                    }
                    result
                }
                if (tapped == true) opener.open(link)
                if (tapped == null) {
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    selected = link
                    do {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        event.changes.forEach { it.consume() }
                    } while (event.changes.any { it.pressed })
                }
            }
        },
        style = style,
        onTextLayout = { layout = it },
    )
    selected?.let { link -> LinkActions(link) { selected = null } }
}

private fun linkAt(layout: TextLayoutResult, spans: List<RichSpan>, position: Offset): RichLink? {
    val line = layout.getLineForVerticalPosition(position.y)
    return if (outsideLine(layout, line, position)) null else spanLinkAt(layout, spans, position)
}

private fun outsideLine(layout: TextLayoutResult, line: Int, position: Offset): Boolean {
    val vertical = position.y >= 0 && position.y <= layout.size.height
    val horizontalStart = position.x >= layout.getLineLeft(line)
    val horizontalEnd = position.x <= layout.getLineRight(line)
    return !vertical || !horizontalStart || !horizontalEnd
}

private fun spanLinkAt(layout: TextLayoutResult, spans: List<RichSpan>, position: Offset): RichLink? {
    val offset = layout.getOffsetForPosition(position)
    var start = 0
    var result: RichLink? = null
    for (span in spans) {
        val end = start + span.text.length
        if (result == null && offset in start until end) result = span.link
        start = end
    }
    return result
}

@Composable
private fun LinkActions(link: RichLink, onDismiss: () -> Unit) {
    val opener = LocalLinkOpener.current
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val label = stringResource(R.string.message_clip_label)
    WontonActionSheet(
        listOf(
            WontonSheetAction(stringResource(R.string.message_copy), WontonIcon.Copy) {
                scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(label, link.url))) }
            },
            WontonSheetAction(stringResource(R.string.message_link_browser), WontonIcon.Globe) {
                opener.openInBrowser(link)
            },
            WontonSheetAction(stringResource(R.string.message_image_share), WontonIcon.Share) { opener.share(link) },
        ),
        onDismiss,
    )
}
