package app.wonton.ui

/**
 * 保活但不在核心页面栈里的网页不应向核心报告加载结果；重开时补成功或重新加载。
 * WebView 的回调可能在页面离栈后到达，所以这份状态不依赖 Compose 的可见性。
 */
internal class KeepAlivePageLifecycle {
    enum class ResumeAction {
        NONE,
        LOAD_SUCCEEDED,
        RELOAD,
    }

    var parked = false
        private set

    private var missedFinish = false
    private var failedWhileParked = false

    fun setParked(value: Boolean) {
        parked = value
    }

    /** 返回 true 表示调用方可以把成功报告给核心。 */
    fun loadFinished(): Boolean {
        if (!parked) return true
        missedFinish = true
        failedWhileParked = false
        return false
    }

    /** 返回 true 表示调用方可以把失败报告给核心。 */
    fun loadFailed(): Boolean {
        if (!parked) return true
        missedFinish = false
        failedWhileParked = true
        return false
    }

    fun resume(): ResumeAction {
        if (parked) return ResumeAction.NONE
        val action = when {
            failedWhileParked -> ResumeAction.RELOAD
            missedFinish -> ResumeAction.LOAD_SUCCEEDED
            else -> ResumeAction.NONE
        }
        missedFinish = false
        failedWhileParked = false
        return action
    }
}
