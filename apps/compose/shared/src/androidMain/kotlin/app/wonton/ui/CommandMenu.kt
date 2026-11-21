package app.wonton.ui

import app.wonton.protocol.RoomCommand

/**
 * 输入框开头敲了“/”时，斜杠后面当过滤用的那段（ADR-0109）：名字里可以有空格，打完一条命令再打参数就不匹配了。
 * 没敲“/”、已经换行或草稿就是刚点出去的那句时为空。
 */
internal fun commandQuery(draft: String, sentSlash: String): String? {
    if (draft == sentSlash) return null
    val filtering = draft.startsWith("/") && draft.none { it == '\n' || it == '\r' }
    return if (filtering) draft.removePrefix("/") else null
}

/**
 * 输入框上方这会儿该列的命令：敲了“/”按名字前缀过滤，菜单键打开的是整张表，都不是就不列。
 * 过滤不看菜单键开没开，所以敲着“/”按菜单键列表不闪掉。
 */
internal fun commandList(
    draft: String,
    menuOpen: Boolean,
    sentSlash: String,
    commands: List<RoomCommand>,
): List<RoomCommand> {
    val query = commandQuery(draft, sentSlash) ?: return if (menuOpen) commands else emptyList()
    return commands.filter { it.name.startsWith(query, ignoreCase = true) }
}

/** 改了草稿以后菜单键打开的那张表还开不开：打了字就收起，清空时留着（照 iOS）。 */
internal fun commandMenuAfterEdit(menuOpen: Boolean, draft: String): Boolean =
    menuOpen && draft.isEmpty()
