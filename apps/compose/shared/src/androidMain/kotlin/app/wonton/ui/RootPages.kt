package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import app.wonton.protocol.Action
import app.wonton.protocol.HomeStatus
import app.wonton.protocol.HomeViewModel
import app.wonton.protocol.PageAction
import app.wonton.protocol.TabRef

/**
 * 主界面最外面这层叠着的页：设置、应用网格、整页的插件网页，垫底的是聊天或应用网格的根。
 * 原生的几页由 `NativeContent` 画，网页由 [PluginPagesHost] 画；返回手势拖动时下面那页从哪边画就在哪边垫出来。
 */
internal sealed interface RootPage {
    /** 认页、存滚动位置用，要能存进 Bundle。 */
    val key: String

    /** 这一页归原生层画（不是插件网页）。 */
    val native: Boolean
        get() = this !is Web

    data object Settings : RootPage { override val key = "settings" }
    data object Apps : RootPage { override val key = "apps" }
    data object Chat : RootPage { override val key = "chat" }
    data class Web(val tab: TabRef) : RootPage { override val key = "web:$tab" }
}

private fun TabRef.rootPage(): RootPage = when (this) {
    TabRef.Chat -> RootPage.Chat
    TabRef.Apps -> RootPage.Apps
    else -> RootPage.Web(this)
}

/**
 * 现在叠着的页，最上面的在前：设置盖在什么上面都算；应用网格（核心的 `showsApps`）盖在页面栈上；
 * 页面栈从栈顶往下，最后是核心的根页。没配对时聊天根换成应用网格（配对页）。
 */
internal fun AppNavigation.rootPages(home: HomeViewModel): List<RootPage> {
    val view = pages.viewModel
    val notConfigured = home.status is HomeStatus.NotConfigured
    val stacked = buildList {
        if (settingsOverlay) add(RootPage.Settings)
        if (view.showsApps) add(RootPage.Apps)
        view.stack.asReversed().forEach { add(it.tab.rootPage()) }
        add(view.root.rootPage())
    }.map { if (notConfigured && it == RootPage.Chat) RootPage.Apps else it }
    return stacked.filterIndexed { index, page -> index == 0 || stacked[index - 1] != page }
}

/** 关掉最上面这层：设置直接收起；网格、页面和栈上的根页交给核心弹栈（异步，换页由核心的下一次更新带来）。 */
internal fun AppNavigation.closeRoot(top: RootPage) {
    when {
        top == RootPage.Settings -> settingsOverlay = false
        pages.viewModel.showsApps -> closeApps()
        pages.viewModel.stack.isNotEmpty() -> pages.send(Action.Pages(PageAction.ClosePage))
    }
}

/** 网页那层要的返回手势上下文：共用最外层的预览，返回后露出 [under]。 */
internal class RootBack(val preview: BackPreview<RootPage>, val under: RootPage?, val currentTop: () -> RootPage)

/**
 * 最外层的返回（原生的设置、应用网格和栈上的根页）。网页自己接返回（它要先走网页历史），这里不管。
 * 装在页面里各层的返回之前，里面那层能返回时先走里面。
 */
@Composable
internal fun rememberRootBack(nav: AppNavigation, chain: List<RootPage>): RootBack {
    val preview = remember { BackPreview<RootPage>() }
    val top = chain.first()
    val under = chain.getOrNull(1)
    val current by rememberUpdatedState(top)
    PredictiveBack(preview, top, under.takeIf { top.native }) { nav.closeRoot(top) }
    return RootBack(preview, under, { current })
}

/**
 * 最外层原生的几页：平时只有最上面那页（是原生页时）；拖返回时把下面那页也垫出来。
 * 最上面是网页、下面是原生页时，这一块整个垫在网页下面（调用方给 zIndex）。
 */
@Composable
internal fun RootNativePages(back: RootBack, top: RootPage, page: @Composable (RootPage) -> Unit) {
    val preview = back.preview
    val under = preview.under?.takeIf { it != top }
    Box(Modifier.fillMaxSize()) {
        // 下面是网页（它在这一块底下）、上面是原生页：压暗盖在网页上、原生页下。
        if (under != null && !under.native && top.native) BackUnderDim(preview)
        BackPreviewPages(preview, listOfNotNull(under?.takeIf { it.native }, top.takeIf { it.native }),
            RootPage::key, page)
    }
}
