package app.wonton.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.client.ClientCore
import app.wonton.protocol.TimelineMessage
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 群聊组尾气泡旁边的发送者头像（ADR-0109）。没有头像图片时画名字的首字，取不到名字就画空圈。 */
@Composable
internal fun MessageSenderAvatar(message: TimelineMessage, core: ClientCore, modifier: Modifier = Modifier) {
    val colors = WontonTheme.colors
    val avatar = message.senderAvatar
    val loaded = avatar?.let { media -> loadedImage(core, media) }
    val image = loaded?.value?.getOrNull()
    Box(
        modifier.size(WontonTheme.size.messageAvatar).clip(CircleShape)
            .background(colors.surfaceSubtle.toColor())
            .testTag("sender-avatar"),
        contentAlignment = Alignment.Center,
    ) {
        val initial = message.senderName.firstOrNull()?.uppercase()
        if (image != null) {
            Image(image, message.senderName, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        } else if (initial != null) {
            Text(initial, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
        }
    }
}

/** 群聊里别人气泡左边的那一格：只有一组最后一条带头像，其余留白。 */
@Composable
internal fun SenderAvatarCell(message: TimelineMessage, core: ClientCore, joinsNext: Boolean) {
    Box(Modifier.width(WontonTheme.size.messageAvatar + WontonTheme.spacing.s4)) {
        if (!joinsNext) MessageSenderAvatar(message, core, Modifier.align(Alignment.BottomStart))
    }
}

/** 一组的第一条上面写发送者名字；名字再长也只占一行，不把气泡撑宽。 */
@Composable
internal fun SenderNameLine(name: String, modifier: Modifier = Modifier) {
    Text(
        name,
        modifier,
        style = WontonTheme.typography.caption,
        color = WontonTheme.colors.textSecondary.toColor(),
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}
