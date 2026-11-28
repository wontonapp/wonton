package app.wonton.android

import android.content.Context
import app.wonton.protocol.Action
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef
import app.wonton.ui.send
import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertNull

internal class PanelPageFixture(context: Context) {
    private val core = (context.applicationContext as WontonApplication).core.getOrThrow()
    private val directory = File(context.cacheDir, "panel-gesture-fixture")
    private val input = File(directory, "pages.json")

    fun reset() {
        check(directory.deleteRecursively()) { "Cannot clear this test's fixture directory" }
        check(directory.mkdirs()) { "Cannot create this test's fixture directory" }
        core.resetUiFakePagesFixture(input.absolutePath)
    }

    fun uninstallPlayer() = updatePlayer("uninstalling", removeUrl = false)

    fun removePlayer() = updatePlayer("page_removed", removeUrl = true)

    private fun updatePlayer(state: String, removeUrl: Boolean) {
        val model = JSONObject(input.readText())
        val panel = model.getJSONArray("panel")
        var player: JSONObject? = null
        for (index in 0 until panel.length()) {
            val page = panel.getJSONObject(index).getJSONObject("page")
            if (page.getString("title") == "Player") {
                player = page
                break
            }
        }
        val page = checkNotNull(player) { "Player is missing from the fixture" }
        if (removeUrl) page.put("url", JSONObject.NULL)
        page.put("state", JSONObject().put("state", state))
        input.writeText(model.toString())
        core.applyUiFakePagesFixture(input.absolutePath)
    }

    fun openPage(tab: TabRef) {
        assertNull(core.send(Action.Pages(PageAction.OpenPage(tab))))
    }
}
