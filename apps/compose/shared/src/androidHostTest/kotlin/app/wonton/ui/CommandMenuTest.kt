package app.wonton.ui

import app.wonton.protocol.RoomCommand
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 命令列表的开关和过滤（ADR-0109）：草稿有字时按菜单键打开整张表、草稿原样不动，敲“/”按名字前缀过滤。
 */
class CommandMenuTest {
    private val help = command("help")
    private val new = command("new")
    private val photo = command("photo album")
    private val commands = listOf(help, new, photo)

    private fun command(name: String) = RoomCommand(
        botUserId = "@bot:home.example",
        botName = "Bot",
        name = name,
        description = name,
        hint = null,
        text = "/$name",
    )

    /** 草稿有字（不以“/”开头）时按菜单键：整张表出来，草稿不再被垫上“/”。 */
    @Test
    fun menuKeyShowsWholeListWithPlainDraft() {
        assertEquals(commands, commandList("你好", menuOpen = true, sentSlash = "", commands))
    }

    /** 再按一次菜单键收起，草稿照旧不匹配。 */
    @Test
    fun menuKeyAgainClosesList() {
        assertTrue(commandList("你好", menuOpen = false, sentSlash = "", commands).isEmpty())
        assertTrue(commandList("", menuOpen = false, sentSlash = "", commands).isEmpty())
    }

    /** 敲着“/”时点菜单键，列表不闪掉：过滤照旧，菜单键开不开都给同一批。 */
    @Test
    fun menuKeyKeepsFilteringSlashDraft() {
        assertEquals(listOf(help), commandList("/he", menuOpen = true, sentSlash = "", commands))
        assertEquals(listOf(help), commandList("/he", menuOpen = false, sentSlash = "", commands))
    }

    /** 过滤不看大小写，名字里带空格的照样按整句匹配。 */
    @Test
    fun queryIgnoresCaseAndKeepsSpacesInNames() {
        assertEquals(listOf(photo), commandList("/PHOTO", menuOpen = false, sentSlash = "", commands))
        assertEquals(listOf(photo), commandList("/photo a", menuOpen = false, sentSlash = "", commands))
    }

    /** “/”后面接了换行的参数算普通草稿：菜单键给整张表，也没东西可过滤。 */
    @Test
    fun newlineEndsQueryAndMenuKeyShowsAll() {
        assertNull(commandQuery("/new\n参数", sentSlash = ""))
        assertEquals(commands, commandList("/new\n参数", menuOpen = true, sentSlash = "", commands))
        assertTrue(commandList("/new\n参数", menuOpen = false, sentSlash = "", commands).isEmpty())
    }

    /** 点了一条命令，草稿还是那句时列表不再按它弹；动一个字再按菜单键才出整张表。 */
    @Test
    fun pickedCommandStaysQuietUntilDraftMoves() {
        assertTrue(commandList("/new", menuOpen = false, sentSlash = "/new", commands).isEmpty())
        assertEquals(listOf(new), commandList("/ne", menuOpen = false, sentSlash = "/new", commands))
    }

    /** 敲字收起菜单键打开的整张表；把输入框清空时留着。 */
    @Test
    fun typingClosesButtonMenuButClearingKeepsIt() {
        assertFalse(commandMenuAfterEdit(menuOpen = true, draft = "你好a"))
        assertTrue(commandMenuAfterEdit(menuOpen = true, draft = ""))
    }

    /** 没有命令的房间里什么都不列；只有“/”时整张表（空）也照样。 */
    @Test
    fun emptyCommandListHasNothingToList() {
        assertTrue(commandList("/", menuOpen = true, sentSlash = "", commands = emptyList()).isEmpty())
        assertEquals(commands, commandList("/", menuOpen = false, sentSlash = "", commands))
    }
}
