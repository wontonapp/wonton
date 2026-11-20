extension ChatModel {
    /// 等房间或等对方出现的私聊落地以后打开；失败时把原因留在聊天页。
    func openAwaitedChat() {
        if let room = awaitedRoom {
            if viewModel.rooms.contains(where: { $0.roomId == room && $0.membership == .joined }) {
                awaitedRoom = nil
                path = [.room(room)]
            } else if viewModel.problem != nil {
                awaitedRoom = nil
                openingFailed = true
            }
            return
        }
        guard let peer = awaitedPeer else { return }
        if let room = viewModel.rooms.first(where: { $0.directUserId == peer && $0.membership == .joined }) {
            awaitedPeer = nil
            show(room.roomId, from: awaitedFrom)
        } else if viewModel.problem != nil {
            awaitedPeer = nil
            openingFailed = true
        }
    }
}
