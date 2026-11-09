package app.wonton.ui

import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.RoomRow
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSheet

/** 转发先选房间：列的是聊天列表里已经加入、能发的房间，选完由调用方发出去并打开。 */
@Composable
fun ForwardSheet(rooms: List<RoomRow>, onPick: (RoomRow) -> Unit, onDismiss: () -> Unit) {
    WontonSheet(onDismiss, stringResource(R.string.message_forward_title)) { close ->
        if (rooms.isEmpty()) {
            WontonRow(stringResource(R.string.message_forward_empty))
        } else {
            LazyColumn(Modifier.heightIn(max = MAX_HEIGHT)) {
                itemsIndexed(rooms, key = { _, row -> row.roomId }) { index, row ->
                    if (index > 0) WontonSeparator()
                    WontonRow(row.name, onClick = { close { onPick(row) } })
                }
            }
        }
    }
}

private val MAX_HEIGHT = 360.dp
