package app.wonton.ui

import app.wonton.protocol.DefaultBotVisibility
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.PluginKind
import app.wonton.protocol.PluginRuntime
import app.wonton.protocol.PluginState
import app.wonton.protocol.PluginTrust
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class PluginEntriesTest {
    private fun plugin(
        visibility: DefaultBotVisibility?,
        bot: String?,
        kind: PluginKind = PluginKind.AGENT,
    ) = InstalledPlugin(
        id = "app.bot",
        title = "Bot",
        summary = "",
        version = "1.0.0",
        revision = "r1",
        kind = kind,
        runtime = PluginRuntime.COMPOSE,
        icon = null,
        trust = PluginTrust.COMMUNITY,
        source = null,
        provider = null,
        viaIndex = null,
        writtenBy = null,
        status = PluginState.Stopped,
        webUrl = null,
        linkUrl = null,
        crossOriginIsolated = false,
        declaresTools = false,
        tabs = emptyList(),
        chat = null,
        botUserId = bot,
        defaultBotVisibility = visibility,
        upgrade = null,
        removable = true,
        notices = emptyList(),
        enabledRootPermissions = emptyList(),
        installProgress = null,
        installFailure = null,
        tunnelPorts = emptyList(),
        matrixWidget = null,
        botShared = false,
        botDanger = false,
        botDangerCanToggle = false,
    )

    private fun hasChat(entries: List<PluginEntry>) = entries.any { it is PluginEntry.Chat }

    /// 身份看不见（absent）但确定有个 Bot：私聊点得开，房间行会写不能聊的原因。
    @Test
    fun absentAgentKeepsItsChatEntry() {
        assertEquals(
            PluginEntry.Chat("@private:home"),
            plugin(DefaultBotVisibility.ABSENT, "@private:home").entries.first(),
        )
    }

    @Test
    fun visibleAgentKeepsItsChatEntry() {
        assertEquals(
            PluginEntry.Chat("@visible:home"),
            plugin(DefaultBotVisibility.VISIBLE, "@visible:home").entries.first(),
        )
    }

    /// 核实不了（unknown）不给私聊入口；没有 id 或不是 Agent 也不给。
    @Test
    fun unknownVisibilityOrMissingIdHasNoChatEntry() {
        assertFalse(hasChat(plugin(DefaultBotVisibility.UNKNOWN, "@unsure:home").entries))
        assertFalse(hasChat(plugin(null, "@null:home").entries))
        assertFalse(hasChat(plugin(DefaultBotVisibility.ABSENT, null).entries))
        assertFalse(hasChat(plugin(DefaultBotVisibility.ABSENT, "").entries))
        assertFalse(hasChat(plugin(DefaultBotVisibility.ABSENT, "@app:home", PluginKind.APP).entries))
    }
}
