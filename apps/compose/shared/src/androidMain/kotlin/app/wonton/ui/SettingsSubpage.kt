package app.wonton.ui

/** 设置导航栈；服务器的二级页返回服务器，关于的二级页返回关于，其余页面返回首页。 */
enum class SettingsSubpage {
    MAIN, BACKUP, PROFILE, MY_APPS, APPEARANCE, NOTIFICATIONS, APP_LOCK, ABOUT, PRIVACY, LICENSES,

    SERVER, CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, REGISTRY_MIRROR, CORE_UPGRADE, PEERS, FRIENDS;

    fun parent(): SettingsSubpage = when (this) {
        CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, REGISTRY_MIRROR, CORE_UPGRADE -> SERVER
        PRIVACY, LICENSES -> ABOUT
        else -> MAIN
    }

    /** 这一页和它上面各级，从这一页排到首页。 */
    fun chain(): List<SettingsSubpage> =
        generateSequence(this) { page -> page.parent().takeIf { page != MAIN } }.toList()
}
