import WontonClient

extension ChatModel {
    /// 退出编辑态后把核心恢复的草稿整份读回来：正文和回复都盖掉本地那份。
    func reloadDraft(_ target: TimelineTarget) async {
        let key = draftKey(target)
        let stored: ComposerDraft?
        do {
            stored = try await core.draft(target: target)
        } catch {
            return
        }
        let restored = stored?.body ?? ""
        suppressNextTyping = drafts[key] != restored
        drafts[key] = restored
        draftStore.setReply(stored?.reply, in: key)
    }

    /// 进入编辑态：先把原文换进输入框、收掉回复，再让核心存下原草稿（ADR-0109 第 10 节）。
    func startEdit(_ text: String, itemId: String, in target: TimelineTarget) {
        draftStore.selectServer(core.servers().servers.first(where: \.current)?.id)
        suppressNextTyping = drafts[draftKey(target)] != text
        drafts[draftKey(target)] = text
        draftStore.setReply(nil, in: draftKey(target))
        replyFocusRevision += 1
        send(.timeline(action: .setTyping(target: target, typing: false)))
        send(.timeline(action: .startEdit(target: target, itemId: itemId)))
    }

    /// 把输入框的变化报给核心；进编辑态那一次（换进来的是原文）已经报过停，跳过。
    func reportTyping(_ typing: Bool, in target: TimelineTarget) {
        if suppressNextTyping {
            suppressNextTyping = false
            return
        }
        send(.timeline(action: .setTyping(target: target, typing: typing)))
    }
}
