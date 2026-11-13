package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.runtime.compositionLocalOf
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.MatrixTarget
import app.wonton.protocol.RichLink
import app.wonton.protocol.RichLinkKind
import app.wonton.protocol.TabRef
import app.wonton.protocol.CoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/** 打开消息里的一条链接。类别由核心判断，壳只按类别开，不自己看地址（ADR-0065）。 */
interface LinkOpener {
    fun open(link: RichLink)
    fun openInBrowser(link: RichLink)
    fun share(link: RichLink)
}

/** 房间页把打开方式放进组合，富文本在时间线很深的地方，不一层层传。 */
val LocalLinkOpener = compositionLocalOf<LinkOpener> { error("chat must provide a LinkOpener") }

/** 外部网页换 Custom Tabs，插件进插件页，Matrix 在 App 里跳，邮件、电话和其他地址交给系统。 */
class AndroidLinkOpener(
    private val context: Context,
    private val core: ClientCore,
    private val navigation: ChatNavigation,
    private val scope: CoroutineScope,
    private val onReject: (Problem) -> Unit,
) : LinkOpener {
    override fun open(link: RichLink) {
        when (val kind = link.kind) {
            is RichLinkKind.External, RichLinkKind.Other -> customTab(link.url)
            is RichLinkKind.Plugin ->
                core.send(Action.OpenPage(TabRef.Plugin(kind.pluginId, null)))?.let(onReject)
            is RichLinkKind.Matrix -> when (val target = kind.target) {
                is MatrixTarget.User -> navigation.openDirectChat(core, target.userId)?.let(onReject)
                is MatrixTarget.Room -> scope.launch {
                    try {
                        val roomId = core.resolveChatRoom(input = target.roomId)
                        navigation.openWhenJoined(core, roomId)
                    } catch (failure: CoreException) {
                        onReject(failure.problem)
                    }
                }
            }
            RichLinkKind.Email, RichLinkKind.Phone -> system(link.url)
        }
    }
    override fun openInBrowser(link: RichLink) = start(Intent(Intent.ACTION_VIEW, Uri.parse(link.url)))

    override fun share(link: RichLink) {
        val intent = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, link.url)
        start(Intent.createChooser(intent, null), R.string.message_link_share_failed)
    }


    /// 外部网页和认不出的地址都按外部开：网页走 Custom Tabs，没有浏览器就用普通浏览器；`wonton://` 这类只有
    /// 别的 App 认识的协议直接交给系统。
    private fun customTab(url: String) {
        val uri = Uri.parse(url)
        if (uri.scheme == "http" || uri.scheme == "https") {
            try {
                CustomTabsIntent.Builder().build().launchUrl(context, uri)
                return
            } catch (_: ActivityNotFoundException) {
                // A browser may still handle ACTION_VIEW without supporting Custom Tabs.
            } catch (failure: SecurityException) {
                onReject(Problem(titleRes = R.string.message_link_open_failed, detail = failure.message))
                return
            }
        }
        start(Intent(Intent.ACTION_VIEW, uri))
    }

    private fun system(url: String) {
        val uri = Uri.parse(url)
        val intent = when (uri.scheme) {
            "mailto" -> Intent(Intent.ACTION_SENDTO, uri)
            "tel" -> Intent(Intent.ACTION_DIAL, uri)
            else -> Intent(Intent.ACTION_VIEW, uri)
        }
        start(intent)
    }

    private fun start(intent: Intent, failureTitle: Int = R.string.message_link_open_failed) {
        try {
            context.startActivity(intent)
        } catch (failure: ActivityNotFoundException) {
            onReject(Problem(titleRes = failureTitle, nextStepRes = R.string.message_link_no_handler,
                detail = failure.message))
        } catch (failure: SecurityException) {
            onReject(Problem(titleRes = failureTitle, detail = failure.message))
        }
    }
}
