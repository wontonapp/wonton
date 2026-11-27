package app.wonton.ui

import app.wonton.protocol.Action
import app.wonton.protocol.BotRef
import app.wonton.protocol.PluginAction
import app.wonton.protocol.StoreConfirmation

/** 插件这一层（[PluginPages]）叠着的页；[PluginNavigation.pluginPages] 从上往下排，返回逐层关掉。 */
internal sealed interface PluginPage {
    /** 认页、存滚动位置用，要能存进 Bundle；同一类页一个（换内容时原地更新，和换页前一样）。 */
    val key: String

    data class StoreConfirm(val confirmation: StoreConfirmation) : PluginPage {
        override val key = "store-confirm"
    }
    data class Contacts(val bot: BotRef) : PluginPage { override val key = "contacts" }
    data class Settings(val target: PluginSettingsTarget) : PluginPage { override val key = "settings" }
    data class Confirm(val confirmation: PluginConfirmation) : PluginPage { override val key = "confirm" }
    data object AddingSource : PluginPage { override val key = "add-source" }
    data class ChangingVersion(val id: String) : PluginPage { override val key = "version" }
    data class Detail(val route: PluginRoute) : PluginPage { override val key = "detail" }

    /** 这一层下面的列表（聊天列表、应用网格、商店）。 */
    data object Base : PluginPage { override val key = "base" }
}

/** 现在叠着的页，最上面的在前；顺序就是显示的优先级，关掉最上面那页，露出来的是下一项。 */
internal fun PluginNavigation.pluginPages(store: StoreConfirmation?): List<PluginPage> = buildList {
    store?.let { add(PluginPage.StoreConfirm(it)) }
    contacts?.let { add(PluginPage.Contacts(it)) }
    settings?.let { add(PluginPage.Settings(it)) }
    confirmation?.let { add(PluginPage.Confirm(it)) }
    if (addingSource) add(PluginPage.AddingSource)
    changingVersion?.let { add(PluginPage.ChangingVersion(it)) }
    detail?.let { add(PluginPage.Detail(it)) }
    add(PluginPage.Base)
}

internal fun PluginNavigation.close(page: PluginPage) {
    when (page) {
        is PluginPage.Contacts -> contacts = null
        is PluginPage.Settings -> settings = null
        is PluginPage.Confirm -> confirmation = null
        PluginPage.AddingSource -> addingSource = false
        is PluginPage.ChangingVersion -> changingVersion = null
        is PluginPage.Detail -> detail = null
        // 核心的那条由 [dropStoreConfirmation] 撤。
        is PluginPage.StoreConfirm, PluginPage.Base -> Unit
    }
}

/** 主人自己关掉核心排来的确认：只撤还在的那条，确认后落进来的下一条不受影响。 */
internal fun PluginsState.dropStoreConfirmation(shown: StoreConfirmation) {
    if (viewModel.confirmation?.preview?.id == shown.preview.id) {
        send(Action.Plugins(PluginAction.DismissStoreConfirmation))
    }
}
