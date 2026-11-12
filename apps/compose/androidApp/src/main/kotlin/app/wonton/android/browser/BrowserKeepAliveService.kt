package app.wonton.android.browser

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder

/**
 * 要跨源隔离的插件页在 Custom Tabs 里打开时，浏览器绑定这个服务（`android.support.customtabs.extra.KEEP_ALIVE`），
 * App 的进程跟着浏览器保持前台的优先级：页面开着，回环入口和隧道就在（ADR-0033）。服务本身什么也不做。
 */
class BrowserKeepAliveService : Service() {
    private val binder = Binder()

    override fun onBind(intent: Intent?): IBinder = binder
}
