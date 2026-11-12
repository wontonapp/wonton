package app.wonton.ui

/**
 * 聊天页上和旁聊有关的入口（ADR-0093）。[supported]：房间里有支持声明，顶栏给开旁聊的按钮；[covered]：旁聊页盖在
 * 这条主对话上；[composing]：开旁聊的面板开着；[unsupportedHint]：“新建聊天”选的 AI 不支持旁聊，打开的是主对话。
 * [onOpen] 的参数是旁聊根的事件 ID。旁聊页自己用 [NONE]：旁聊里不再开旁聊，也没有卡片。
 */
internal class SideChatEntry(
    val supported: Boolean,
    val covered: Boolean,
    val composing: Boolean,
    val unsupportedHint: Boolean,
    val onOpen: (String) -> Unit,
    val onComposingChange: (Boolean) -> Unit,
) {
    companion object {
        val NONE = SideChatEntry(
            supported = false,
            covered = false,
            composing = false,
            unsupportedHint = false,
            onOpen = {},
            onComposingChange = {},
        )
    }
}
