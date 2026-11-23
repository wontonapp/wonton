// 好友页（ADR-0088 / ADR-0051）：只有好友行和请求，没有分组。
package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.FriendRequest
import app.wonton.protocol.FriendRequestKind
import app.wonton.protocol.FriendRow
import app.wonton.protocol.FriendsState
import app.wonton.protocol.FriendsViewModel

private fun previewFriend(name: String, user: String, shared: List<String>) = FriendRow(
    userId = user,
    displayName = name,
    roleId = "friend",
    sharedPlugins = shared,
    lastOpenedShareAtMs = null,
)

private fun previewFriends(): FriendsViewModel = FriendsViewModel(
    state = FriendsState.Ready,
    friends = listOf(
        previewFriend("Anna", "@anna:example.test", listOf("app.photos", "app.notes")),
        previewFriend("Marco", "@marco:example.test", emptyList()),
        previewFriend("Lena", "@lena:example.test", listOf("app.notes")),
    ),
    incoming = listOf(
        FriendRequest(
            roomId = "!request:home.example",
            userId = "@sam:example.test",
            displayName = "Sam",
            kind = FriendRequestKind.FRIEND,
            invitedAtMs = 1_793_000_000_000u,
        ),
    ),
    outgoing = emptyList(),
    blocked = emptyList(),
    canBlockFederation = true,
    publicAccessOff = false,
    myLink = null,
)

@Preview(name = "好友页", locale = "en", widthDp = 390, heightDp = 700)
@Composable
internal fun FriendsPreview() {
    PreviewFrame {
        FriendList(previewFriends(), null, {}, {}, {}, {}, {})
    }
}

@Preview(name = "好友页 深色", locale = "en", widthDp = 390, heightDp = 700)
@Composable
internal fun FriendsDarkPreview() {
    PreviewFrame(dark = true) {
        FriendList(previewFriends(), null, {}, {}, {}, {}, {})
    }
}
