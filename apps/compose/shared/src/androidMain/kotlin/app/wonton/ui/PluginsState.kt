package app.wonton.ui

import android.graphics.ImageDecoder
import android.util.LruCache
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.PluginImage
import app.wonton.protocol.PluginKind
import app.wonton.protocol.PluginState
import app.wonton.protocol.InstallPreview
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.PluginsViewModel
import app.wonton.protocol.StoreConfirmation
import app.wonton.protocol.StoreItem
import app.wonton.protocol.StoreListing
import app.wonton.protocol.TabRef
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.IOException
import java.nio.ByteBuffer

private const val CACHED_IMAGES = 128

/**
 * 主屏、商店和核心之间的桥：持有核心推来的插件 ViewModel，转发操作，替界面取插件的图片。
 * 这里不做任何业务判断。
 */
class PluginsState(val core: ClientCore) {
    var viewModel by mutableStateOf(core.plugins().also {
        configureCrossOriginIsolatedAllowlist(it.installed?.plugins.orEmpty())
    })
        private set

    /** 核心拒绝主人的操作的原因，由界面弹窗展示。 */
    var rejection by mutableStateOf<Problem?>(null)

    /** 页面出现时自动发的刷新被核心拒绝的原因。不弹窗，没有 `viewModel.problem` 时就地显示。 */
    var refreshRejection by mutableStateOf<CoreException?>(null)
        private set

    /** 核心给的图片地址内容不变（ADR-0017），按地址缓存解码后的图片。 */
    private val images = LruCache<String, ImageBitmap>(CACHED_IMAGES)

    /** 在主界面里一直收集，切到别的页面时也接着更新。 */
    suspend fun observe() {
        core.pluginsUpdates().collect { next ->
            if (next.installed?.plugins != viewModel.installed?.plugins) {
                configureCrossOriginIsolatedAllowlist(next.installed?.plugins.orEmpty())
            }
            viewModel = next
        }
    }

    fun send(action: Action) {
        reject(core.send(action))
    }

    /** 只在有问题时更新提示：成功的操作不清掉还没看的问题。 */
    fun reject(problem: Problem?) {
        problem?.let { rejection = it }
    }

    /** 发页面出现时的刷新，被拒绝的原因记在 [refreshRejection]。 */
    fun refresh(action: Action) {
        refreshRejection = try {
            core.dispatch(action)
            null
        } catch (rejection: CoreException) {
            rejection
        }
    }

    /** 取插件的图标或截图；取不到、解不开时为 null，界面画占位（ADR-0017），不当作错误。 */
    suspend fun imageOrNull(image: PluginImage): ImageBitmap? =
        images.get(image.path) ?: try {
            val bytes = core.loadPluginImage(image)
            withContext(Dispatchers.Default) {
                ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))).asImageBitmap()
            }.also { images.put(image.path, it) }
        } catch (ignored: CoreException) {
            null
        } catch (ignored: IOException) {
            null
        }
}

/** 已安装的插件从主屏和商店打开的方式（ADR-0017 的“主屏入口”）。 */
sealed interface PluginEntry {
    /** 和插件的 Bot 的私聊。 */
    data class Chat(val botUserId: String) : PluginEntry

    data class Web(val tab: TabRef) : PluginEntry
}

/** 打开它的方式，第一种是点图标时用的。都没有时点开看详情。 */
val InstalledPlugin.entries: List<PluginEntry>
    get() = buildList {
        val bot = botUserId
        if (kind == PluginKind.AGENT && bot != null) add(PluginEntry.Chat(bot))
        tabChoices.firstOrNull()?.let { add(PluginEntry.Web(it.tab)) }
    }

val InstalledPlugin.isBusy: Boolean
    get() = when (status) {
        is PluginState.Installing, is PluginState.Upgrading, is PluginState.Uninstalling -> true
        else -> false
    }

internal fun PluginsViewModel.hasRunningTask(id: String): Boolean =
    tasks.any { it.id == id && it.state is PluginTaskState.Running }

/** 插件详情页：商店里的（来源最新的版本）或者装着的。 */
sealed interface PluginRoute {
    data class Store(val source: String) : PluginRoute

    data class Installed(val id: String) : PluginRoute
}

/** 安装或升级的确认页。 */
sealed interface PluginConfirmation {
    data class Install(val source: String) : PluginConfirmation

    data class Upgrade(val id: String) : PluginConfirmation

    /** 主人选的压缩包。 */
    class Upload(val archive: ByteArray) : PluginConfirmation

    /** 换到主人填的来源。 */
    data class UpgradeFrom(val id: String, val source: String) : PluginConfirmation

    /** 换到主人选的压缩包。 */
    class UpgradeUpload(val id: String, val archive: ByteArray) : PluginConfirmation

    /** 核心取好预览、请主人拿主意的一键操作（ADR-0017）：预览不再重新取。 */
    data class Store(val confirmation: StoreConfirmation) : PluginConfirmation

    /** 核心已经取好的预览；确认页不再自己取。 */
    val preloaded: InstallPreview?
        get() = (this as? Store)?.confirmation?.preview

    val isUpgrade: Boolean
        get() = this is Upgrade || this is UpgradeFrom || this is UpgradeUpload ||
            (this is Store && confirmation is StoreConfirmation.Upgrade)

    /** 主人选的版本可能比装着的旧，不叫升级。 */
    val actionTitle: String
        @Composable get() = when (this) {
            is Install, is Upload -> stringResource(R.string.install_action_install)
            is Upgrade -> stringResource(R.string.install_action_upgrade)
            is UpgradeFrom, is UpgradeUpload -> stringResource(R.string.install_change_version)
            is Store -> if (confirmation is StoreConfirmation.Upgrade) {
                stringResource(R.string.install_action_upgrade)
            } else {
                stringResource(R.string.install_action_install)
            }
        }
}

/** 商店里全部的插件，按显示的顺序。 */
val StoreListing.allPlugins: List<StoreItem>
    get() = official.plugins + added.flatMap { it.section.plugins } + community.plugins

/** 要卸载的插件，确认对话框用。 */
data class UninstallTarget(val id: String, val title: String)

/** 要打开设置页的插件，设置页用：标题栏显示插件的标题。 */
data class PluginSettingsTarget(val id: String, val title: String)

/** 核心取好的预览：确认页不再重新取。 */
val StoreConfirmation.preview: InstallPreview
    get() = when (this) {
        is StoreConfirmation.Install -> preview
        is StoreConfirmation.Upgrade -> preview
    }
