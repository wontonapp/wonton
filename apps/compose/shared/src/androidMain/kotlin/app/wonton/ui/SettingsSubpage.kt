package app.wonton.ui

/** 设置导航栈；服务器的二级页返回服务器，其余页面返回首页。 */
enum class SettingsSubpage {
    MAIN, BACKUP, PROFILE, TABS, APPEARANCE, ABOUT,
    SERVER, CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, CORE_UPGRADE;

    fun parent(): SettingsSubpage = when (this) {
        CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, CORE_UPGRADE -> SERVER
        else -> MAIN
    }
}
