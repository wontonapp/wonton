package app.wonton.ui.widget

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.glance.action.Action
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.color.ColorProvider
import androidx.glance.unit.ColorProvider
import app.wonton.client.appLinkUrl
import app.wonton.client.defaultAppearance
import app.wonton.client.readWidgetSnapshot
import app.wonton.protocol.AppLink
import app.wonton.protocol.Rgb
import app.wonton.protocol.ThemeColors
import app.wonton.protocol.WidgetSnapshot
import app.wonton.protocol.WidgetSnapshotRead
import app.wonton.ui.design.toColor
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * 小组件画一次要的全部东西（ADR-0100）：核心写的快照或读不到的原因、颜色、此刻。点了去哪由 [open] 给，界面只画，
 * 不读文件、不调核心。
 */
class WidgetState(
    val read: WidgetSnapshotRead,
    val light: ThemeColors,
    val dark: ThemeColors,
    val nowMs: Long,
    val open: (AppLink) -> Action,
) {
    val snapshot: WidgetSnapshot? get() = (read as? WidgetSnapshotRead.Ready)?.snapshot
    val colors = WidgetColors(light, dark)
}

/** 小组件用的颜色：App 的浅色、深色令牌（ADR-0026），跟着系统明暗。 */
class WidgetColors(private val light: ThemeColors, private val dark: ThemeColors) {
    private fun pick(token: (ThemeColors) -> Rgb): ColorProvider =
        ColorProvider(day = token(light).toColor(), night = token(dark).toColor())

    val surface = pick { it.surface }
    val textPrimary = pick { it.textPrimary }
    val textSecondary = pick { it.textSecondary }
    val accentFill = pick { it.accentFill }
    val accentInk = pick { it.accentInk }
    val accentInkOnFill = pick { it.accentInkOnFill }
    val success = pick { it.success }
    val warning = pick { it.warning }
}

/**
 * 快照又写了一次。小组件的会话还开着时（Glance 留着组合）靠它重读文件；没开着时由 `updateAll` 重开一次。
 */
internal object WidgetRevision {
    private val revision = MutableStateFlow(0L)
    val flow: StateFlow<Long> = revision.asStateFlow()

    fun bump() {
        revision.value += 1
    }
}

/** 从 App 的数据目录读快照。读不了的原因只进日志，界面按“读不到”画。 */
internal fun loadWidgetState(context: Context): WidgetState {
    val read = readWidgetSnapshot(context.filesDir.absolutePath)
    if (read is WidgetSnapshotRead.Unreadable) Log.w(TAG, "widget snapshot unreadable: ${read.detail}")
    // 读不到快照时没有用户选的颜色，用 App 的默认外观。
    val (light, dark) = (read as? WidgetSnapshotRead.Ready)?.snapshot?.let { it.light to it.dark }
        ?: defaultAppearance().let { it.light to it.dark }
    return WidgetState(
        read = read,
        light = light,
        dark = dark,
        nowMs = System.currentTimeMillis(),
        open = { link -> actionStartActivity(appLinkIntent(context, link)) },
    )
}

/** 打开 App 里某一处的 Intent，和桌面快捷方式一样经 `wonton://` 链接进 MainActivity。 */
fun appLinkIntent(context: Context, link: AppLink): Intent =
    Intent(Intent.ACTION_VIEW, Uri.parse(appLinkUrl(link)))
        .setComponent(ComponentName(context.packageName, "${context.packageName}.MainActivity"))
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)

private const val TAG = "WontonWidget"
