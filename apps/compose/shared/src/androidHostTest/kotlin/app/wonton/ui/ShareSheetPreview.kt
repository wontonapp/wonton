// 分享页（ADR-0097）的假数据和预览，和 PreviewData.kt 一样只进测试源集，不连核心、不加载 .so。
package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.AttachmentKind
import app.wonton.protocol.ChatForm
import app.wonton.protocol.ChatLayout
import app.wonton.protocol.ChatTextScale
import app.wonton.protocol.CodeOverflow
import app.wonton.protocol.PendingShare
import app.wonton.protocol.RoomMembership
import app.wonton.protocol.RoomRow
import app.wonton.protocol.ShareInDestinations
import app.wonton.protocol.ShareInItem
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private fun previewShareRoom(id: String, name: String, bot: String?) = RoomRow(
    roomId = id,
    name = name,
    isDirect = bot != null,
    directUserId = bot,
    isEncrypted = true,
    isReadOnly = false,
    membership = RoomMembership.JOINED,
    preview = null,
    sideChats = false,
    unreadCount = 0u,
    layout = ChatLayout(form = ChatForm.BUBBLES, textScale = ChatTextScale.STANDARD, code = CodeOverflow.WRAP),
)

// 路径故意不存在：截图里照片走占位图标，不读磁盘。
private fun previewShareItem(id: String, name: String, image: Boolean) = ShareInItem(
    id = id,
    kind = if (image) AttachmentKind.Image(width = null, height = null) else AttachmentKind.File,
    name = name,
    mimeType = if (image) "image/jpeg" else "application/pdf",
    size = 1_200_000u,
    path = "/nonexistent/$id",
)

private fun previewShare(): PendingShare {
    val agent = previewShareRoom("!agent:home.example", "Agent", "@agent:home.example")
    return PendingShare(
        id = "preview",
        items = listOf(
            previewShareItem("0", "IMG_0001.JPG", image = true),
            previewShareItem("1", "IMG_0002.JPG", image = true),
            previewShareItem("2", "Floor plan.pdf", image = false),
        ),
        text = "Which one looks better?",
        destinations = ShareInDestinations.Ready(
            rooms = listOf(agent, previewShareRoom("!family:home.example", "Family", null)),
            suggestedRoom = agent.roomId,
        ),
        dropped = 2u,
    )
}

@Composable
private fun PreviewRoomAvatar(row: RoomRow) {
    Box(
        Modifier.size(WontonTheme.size.avatar).background(WontonTheme.colors.surfaceSubtle.toColor(), CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Text(row.name.take(1), style = WontonTheme.typography.itemTitle)
    }
}

/** 分享进来三项和一句话：去向是默认 Agent，提示丢了两项。 */
@Preview(name = "分享页", locale = "en", widthDp = 390, heightDp = 420)
@Composable
internal fun ShareSheetPreview() {
    PreviewFrame {
        ShareSheetContent(
            share = previewShare(),
            avatar = { PreviewRoomAvatar(it) },
            sending = false,
            onPair = {},
            onRemove = {},
            onSend = { _, _ -> },
        )
    }
}

/** 还没配对：条目照样在，去向换成配对的引导。 */
@Preview(name = "分享页（没配对）", locale = "en", widthDp = 390, heightDp = 360)
@Composable
internal fun ShareSheetNotPairedPreview() {
    PreviewFrame {
        ShareSheetContent(
            share = previewShare().copy(destinations = ShareInDestinations.NotPaired, dropped = 0u),
            avatar = { PreviewRoomAvatar(it) },
            sending = false,
            onPair = {},
            onRemove = {},
            onSend = { _, _ -> },
        )
    }
}
