package app.wonton.ui

/** 系统返回这一步该做什么；顺序即优先级。 */
internal enum class PluginBackTarget { CloseCustomView, History, ClosePage }

/** 返回键的所有入口（系统返回、胶囊菜单）走完 [PluginWebViewController.back] 后的收尾：要关页就交给调用方（弹栈）。 */
internal fun PluginWebViewController.backOrClose(onClosePage: () -> Unit) {
    if (back() == PluginBackTarget.ClosePage) onClosePage()
}

/** 日志里的地址：只到路径。查询串里可能有 OAuth 授权码，片段是登录跳转的一部分。 */
internal fun logUrl(url: String): String = url.substringBefore('?').substringBefore('#')

/**
 * 当前页前面最近的一步历史。[sameDocument]：走回去看到的还是这一页（同一地址，或一条会自己跳回本页的重定向），
 * 网页上不会有变化。[previous] 只给日志看，经过 [logUrl]。
 */
internal data class HistoryStep(val sameDocument: Boolean, val previous: String? = null)

/**
 * 返回键的去向。网页历史分两种：跨文档的一步照浏览器的语义后退；同一文档里的一步常常只是重定向、replaceState
 * 的记录，读屏上按不出反应，按没有历史处理直接关页。全屏视频先退出，页面还在。
 */
internal fun pluginBackTarget(customView: Boolean, history: HistoryStep?): PluginBackTarget = when {
    customView -> PluginBackTarget.CloseCustomView
    history == null -> PluginBackTarget.ClosePage
    history.sameDocument -> PluginBackTarget.ClosePage
    else -> PluginBackTarget.History
}
