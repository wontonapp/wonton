package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginSettingField
import app.wonton.protocol.PluginSettingsUpdate
import app.wonton.protocol.PluginSettingsView
import app.wonton.protocol.SettingValue
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** 保存的结果：核心返回的最新视图、要不要重读整页，和留给页面显示的问题。 */
private class SaveResult(val refreshed: PluginSettingsView?, val reload: Boolean, val problem: Problem?)

/**
 * 插件设置页（ADR-0077）：按核心给的原生控件子集画表单，只把改动过的项发回去。
 * 核心标出问题的项在控件旁标红；比这个 App 新的控件只读，且整份表单禁止提交。
 * 保存被拒后用预览把核心校验出的字段问题落到控件上；预览也失败（如插件换了设置文件）就重读整页。
 */
@Composable
fun PluginSettingsScreen(plugins: PluginsState, target: PluginSettingsTarget, onClose: () -> Unit) {
    BackHandler(onBack = onClose)
    val id = target.id
    var attempt by remember(id) { mutableIntStateOf(0) }
    val loaded by produceState<Result<PluginSettingsView>?>(null, id, attempt) {
        value = null
        value = try {
            Result.success(plugins.core.pluginSettings(id))
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    // 保存或预览成功后核心返回的最新视图，盖在读接口的结果上。
    var saved by remember(id) { mutableStateOf<PluginSettingsView?>(null) }
    // 改动过的项：键是属性名，值 null 表示清空回默认。
    var edits by remember(id) { mutableStateOf<Map<String, SettingValue?>>(emptyMap()) }
    // 文本类控件的草稿，允许还没填成合法值的中间状态。
    var drafts by remember(id) { mutableStateOf<Map<String, String>>(emptyMap()) }
    var saving by remember(id) { mutableStateOf(false) }
    var failure by remember(id) { mutableStateOf<Problem?>(null) }
    val scope = rememberCoroutineScope()

    WontonScreen(bar = { WontonPushedBar(target.title, onClose, showsStatusLine = true) }) {
        val result = loaded
        val view = saved ?: result?.getOrNull()
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.ime)) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    result == null -> LoadingView()
                    result.isFailure -> ProblemView(result.failureProblem, onRetry = { attempt++ })
                    view != null -> SettingsForm(
                        view, edits, drafts, failure,
                        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wontonContentPadding()
                            .padding(WontonTheme.spacing.pageInset),
                        onEdit = { name, value ->
                            failure = null
                            edits = edits + (name to value)
                        },
                        onDraft = { field, draft ->
                            failure = null
                            drafts = drafts + (field.name to draft)
                            edits = edits + (field.name to draftValue(field, draft))
                        },
                    )
                }
            }
            view?.let { current ->
                // 改不了的项和缺必填值的项挡在前面，核心到时也会再拒一次。
                val blocked = current.form.fields.any { field ->
                    isUnsupported(field, edits) || isMissingRequired(field, edits)
                }
                WontonButton(
                    stringResource(R.string.common_save),
                    {
                        saving = true
                        scope.launch {
                            val outcome = saveSettings(plugins, id, current, edits)
                            if (outcome.reload) {
                                // 设置文件多半在中间换过了（409 changed）：放下草稿和旧视图，重读整页拿新摘要。
                                saved = null
                                edits = emptyMap()
                                drafts = emptyMap()
                                attempt++
                            } else if (outcome.problem == null) {
                                edits = emptyMap()
                                drafts = emptyMap()
                            }
                            outcome.refreshed?.let { saved = it }
                            failure = outcome.problem
                            saving = false
                        }
                    },
                    Modifier.padding(bottom = LocalWontonBottomInset.current)
                        .padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
                    enabled = !saving && !blocked,
                )
            }
        }
    }
}

/** 保存；被拒时预览一次把字段问题带回来。 */
private suspend fun saveSettings(
    plugins: PluginsState,
    id: String,
    view: PluginSettingsView,
    edits: Map<String, SettingValue?>,
): SaveResult = try {
    SaveResult(plugins.core.setPluginSettings(id, PluginSettingsUpdate(view.schemaDigest, edits)), false, null)
} catch (e: CoreException) {
    try {
        SaveResult(
            plugins.core.previewPluginSettings(id, PluginSettingsUpdate(view.schemaDigest, edits)),
            false,
            e.problem,
        )
    } catch (_: CoreException) {
        SaveResult(null, true, e.problem)
    }
}

/** 表单正文：一项一节，有问题的项在控件旁标红。 */
@Composable
private fun SettingsForm(
    view: PluginSettingsView,
    edits: Map<String, SettingValue?>,
    drafts: Map<String, String>,
    failure: Problem?,
    modifier: Modifier = Modifier,
    onEdit: (name: String, value: SettingValue?) -> Unit,
    onDraft: (field: PluginSettingField, draft: String) -> Unit,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        failure?.let { ProblemNotice(it) }
        if (view.form.fields.any { isUnsupported(it, edits) }) {
            WontonNotice(WontonIcon.Warning, stringResource(R.string.plugin_settings_update_app), isError = false)
        }
        view.form.fields.forEach { field ->
            SettingsField(
                field, effectiveValue(field, edits), textDraft(field, edits, drafts),
                disabled = isUnsupported(field, edits),
                missing = isMissingRequired(field, edits),
                onEdit = onEdit,
                onDraft = onDraft,
            )
        }
    }
}
