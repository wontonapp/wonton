package app.wonton.ui

/** 设置导航栈；服务器的二级页返回服务器，关于的二级页返回关于，其余页面返回首页。 */
enum class SettingsSubpage {
    MAIN, BACKUP, PROFILE, MY_APPS, APPEARANCE, NOTIFICATIONS, APP_LOCK, ABOUT, PRIVACY, LICENSES,

    SERVER, CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, REGISTRY_MIRROR, CORE_UPGRADE, PEERS, FRIENDS,
    CLIENTS_DOMAIN;

    fun parent(): SettingsSubpage = when (this) {
        CONNECTION_DETAILS, INVITE, PAIRING, EGRESS, REGISTRY_MIRROR, CORE_UPGRADE -> SERVER
        PRIVACY, LICENSES -> ABOUT
        else -> MAIN
    }
}
