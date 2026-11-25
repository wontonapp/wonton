import Foundation
import WontonClient

extension ChatModel {
    /// 把 `source` 时间线里的一条消息转发到选好的会话（ADR-0073 第 4 节、ADR-0109 第 9 节）。核心一次只转发到一个房间，
    /// 选了几个就各派发一次；附言跟着转发交给核心，它在同一房间先发附言、服务器确认后才发副本，顺序由核心保证。
    /// 只发给一个会话时跟过去看；发给几个就留在原房间。
    func forward(_ itemId: String, from source: TimelineTarget, to rooms: [RoomRow], comment: String) {
        // 壳先 trim 判空白，只含空白当没有；交给核心的已是 trim 后的值，核心还会再 trim 一遍。
        let comment = comment.trimmingCharacters(in: .whitespacesAndNewlines)
        for room in rooms {
            let sent = send(.timeline(action: .forwardMessage(
                target: source,
                itemId: itemId,
                toRoomId: room.roomId,
                comment: comment.isEmpty ? nil : comment
            )))
            walkLog.notice("forward to \(room.roomId, privacy: .public) ok=\(sent, privacy: .public)")
        }
        if rooms.count == 1, let only = rooms.first {
            openRoom(only.roomId)
        }
    }
}
