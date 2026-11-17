package app.wonton.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.PortMappingWarning
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * widget 页（ADR-0096）在网页之外要做的两件事。
 *
 * 驱动发给页面的消息：页面的地址出来、开始加载后订阅，每一段脚本原样注入同一个页面，壳不解析报文；页面发来的
 * widget 报文照常经 `wontonBridge` 交给核心，应答也从这条订阅回来。每次加载（地址或重载次数变了）是一次新的
 * widget 会话，重新订阅；订阅返回空就是会话结束了，核心会跟着收页面或给新地址。
 *
 * 第一版离开通话页就挂断（ADR-0096 第 3 节）：App 退到后台，widget 页跟着关。
 */
@Composable
internal fun WidgetPageEffects(state: WebPageState, page: TabPage, pages: PagesState) {
    val url = page.url ?: return
    LaunchedEffect(state, url, page.reload) {
        val subscription = try {
            pages.core.subscribeTabWidget(state.place)
        } catch (failure: CoreException) {
            state.problem = failure.problem
            return@LaunchedEffect
        }
        subscription.use {
            try {
                while (true) state.controller.evaluateJavascript(it.next() ?: break)
            } finally {
                // 协程取消不保证传到核心：壳自己先拆页面时明说。
                it.cancel()
            }
        }
    }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, state) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP && pages.current == state.tab) {
                pages.send(Action.Pages(PageAction.ClosePage))
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
}

/**
 * 通话页上方的一条轻提示（ADR-0096 第 4 节）：widget 页的媒体走插件的端口映射，映射只连上了公共中继时核心给
 * 提醒的类别，照样能打。别的页面没有。
 */
@Composable
internal fun callRelayHint(tab: TabRef, pages: PagesState): String? {
    val pluginId = (tab as? TabRef.Widget)?.pluginId ?: return null
    val warning = pages.plugins.viewModel.portMappings.firstOrNull { it.pluginId == pluginId }?.warning
    return if (warning == PortMappingWarning.PUBLIC_RELAY_BANDWIDTH) {
        stringResource(R.string.tab_call_relay_hint)
    } else {
        null
    }
}

/** 页面浮层里的一条轻提示，样子同短状态条。 */
@Composable
internal fun PageHint(text: String) {
    Surface(
        Modifier.fillMaxWidth().padding(top = WontonTheme.spacing.s8).testTag("page-hint"),
        shape = RoundedCornerShape(WontonTheme.radii.control),
        color = WontonTheme.colors.surface.toColor(),
    ) {
        Text(
            text, Modifier.padding(WontonTheme.spacing.s8),
            style = WontonTheme.typography.supporting, color = WontonTheme.colors.textPrimary.toColor(),
        )
    }
}
