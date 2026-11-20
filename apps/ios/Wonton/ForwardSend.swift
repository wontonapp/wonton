import WontonClient

extension ChatModel {
    /// 把 `source` 时间线里的一条消息转发到选好的会话（ADR-0073 第 4 节、ADR-0109 第 8 节）。核心一次只转发到一个房间，
    /// 选了几个就各发一次；写了附言的，每个会话先发附言、再转发，照 Telegram 附言在上、转发的消息在下。
    /// 只发给一个会话时跟过去看；发给几个就留在原房间。
    ///
    /// 两个动作在核心里各起一个异步任务，附言和转发谁先进对方房间的发送队列核心不保证；要严格有序得让核心的转发
    /// 带上附言。
    func forward(_ itemId: String, from source: TimelineTarget, to rooms: [RoomRow], comment: String) {
        let comment = comment.trimmingCharacters(in: .whitespacesAndNewlines)
        for room in rooms {
            if !comment.isEmpty {
                let target = TimelineTarget(roomId: room.roomId, threadRoot: nil)
                send(.timeline(action: .sendText(target: target, body: comment, replyTo: nil)))
            }
            send(.timeline(action: .forwardMessage(target: source, itemId: itemId, toRoomId: room.roomId)))
        }
        if rooms.count == 1, let only = rooms.first {
            openRoom(only.roomId)
        }
    }
}
