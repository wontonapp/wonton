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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginSettingField
import app.wonton.protocol.PluginSettingsUpdate
import app.wonton.protocol.PluginSettingsView
import app.wonton.protocol.SettingValue
import app.wonton.protocol.SettingsFieldProblem
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** 改动停下来到发预检之间等多久：太短会一边打字一边请求，太长红字跟不上。 */
private const val PREVIEW_DEBOUNCE_MS = 600L

/** 保存的结果：核心返回的最新视图、要不要重读整页、留给页面显示的问题、这次改动会不会重启应用，和按字段的明细。 */
private class SaveResult(
    val refreshed: PluginSettingsView? = null,
    val reload: Boolean = false,
    val problem: Problem? = null,
    val willRestart: Boolean = false,
    val fieldProblems: Map<String, SettingsFieldProblem> = emptyMap(),
)

/**
 * 插件设置页（ADR-0077）：按核心给的原生控件子集画表单，只把改动过的项发回去。
 * 改动停下来就把候选值发给核心预检，把问题标到控件旁；保存是最终一道门。
 * 比这个 App 新的控件只读，且整份表单禁止提交。保存被拒后用预览把核心校验出的字段问题落到控件上；
 * 预览也失败（如插件换了设置文件）就重读整页。
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
    // 保存或预检成功后核心返回的最新视图，盖在读接口的结果上。
    var saved by remember(id) { mutableStateOf<PluginSettingsView?>(null) }
    // 改动过的项：键是属性名，值 null 表示清空回默认。
    var edits by remember(id) { mutableStateOf<Map<String, SettingValue?>>(emptyMap()) }
    // 文本类控件的草稿，允许还没填成合法值的中间状态。
    var drafts by remember(id) { mutableStateOf<Map<String, String>>(emptyMap()) }
    var saving by remember(id) { mutableStateOf(false) }
    var failure by remember(id) { mutableStateOf<Problem?>(null) }
    // 实时预检的最新结果；只拿来标问题，显示值照旧以本地改动优先。
    var previewed by remember(id) { mutableStateOf<PluginSettingsView?>(null) }
    // 预检或保存被拒时核心按字段给的明细：键是属性名，盖过视图里的 field.problem。
    var fieldProblems by remember(id) { mutableStateOf<Map<String, SettingsFieldProblem>>(emptyMap()) }
    // 保存成功后这次改动会不会重启应用：改动的项里有服务用到。
    var restartHint by remember(id) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(edits) {
        if (edits.isEmpty()) {
            previewed = null
            fieldProblems = emptyMap()
        } else {
            delay(PREVIEW_DEBOUNCE_MS)
            val view = saved ?: loaded?.getOrNull() ?: return@LaunchedEffect
            try {
                previewed = plugins.core.previewPluginSettings(id, PluginSettingsUpdate(view.schemaDigest, edits))
                fieldProblems = emptyMap()
            } catch (failure: CoreException) {
                // 候选值没过校验：明细逐字段标红。预检本身拿不到就维持现状，保存时核心会再拦一次。
                if (failure is CoreException.SettingsInvalid) {
                    fieldProblems = failure.fields.associate {
                        it.name to SettingsFieldProblem(it.kind, it.message)
                    }
                }
            }
        }
    }

    PluginSettingsBody(
        title = target.title,
        onClose = onClose,
        loaded = loaded,
        saved = saved,
        edits = edits,
        drafts = drafts,
        saving = saving,
        failure = failure,
        previewed = previewed,
        fieldProblems = fieldProblems,
        restartHint = restartHint,
        onEdit = { name, value ->
            failure = null
            edits = edits + (name to value)
        },
        onDraft = { field, draft ->
            failure = null
            drafts = drafts + (field.name to draft)
            edits = edits + (field.name to draftValue(field, draft))
        },
        onRetry = { attempt++ },
        onSave = { current, pending ->
            saving = true
            scope.launch {
                val outcome = saveSettings(plugins, id, current, pending)
                if (outcome.reload) {
                    // 设置文件多半在中间换过了（409 changed）：放下草稿和旧视图，重读整页拿新摘要。
                    saved = null
                    edits = emptyMap()
                    drafts = emptyMap()
                    attempt++
                } else if (outcome.problem == null) {
                    restartHint = outcome.willRestart
                    edits = emptyMap()
                    drafts = emptyMap()
                }
                outcome.refreshed?.let { saved = it }
                fieldProblems = outcome.fieldProblems
                failure = outcome.problem
                saving = false
            }
        },
    )
}

/** 正文：按核心给的视图画表单和保存按钮；状态在外面，这里只渲染和把操作发回去。 */
@Composable
private fun PluginSettingsBody(
    title: String,
    onClose: () -> Unit,
    loaded: Result<PluginSettingsView>?,
    saved: PluginSettingsView?,
    edits: Map<String, SettingValue?>,
    drafts: Map<String, String>,
    saving: Boolean,
    failure: Problem?,
    previewed: PluginSettingsView?,
    fieldProblems: Map<String, SettingsFieldProblem>,
    restartHint: Boolean,
    onEdit: (name: String, value: SettingValue?) -> Unit,
    onDraft: (field: PluginSettingField, draft: String) -> Unit,
    onRetry: () -> Unit,
    onSave: (current: PluginSettingsView, pending: Map<String, SettingValue?>) -> Unit,
) {
    WontonScreen(bar = { WontonPushedBar(title, onClose, showsStatusLine = true) }) {
        val result = loaded
        val base = saved ?: result?.getOrNull()
        // 预检结果盖在基础视图上：字段问题换成核心按候选值算的，控件的显示值仍以本地改动优先。
        val displayed = previewed ?: base
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.ime)) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    result == null -> LoadingView()
                    result.isFailure -> ProblemView(result.failureProblem, onRetry = onRetry)
                    displayed != null -> SettingsForm(
                        displayed, edits, drafts, failure, restartHint, fieldProblems,
                        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wontonContentPadding()
                            .padding(WontonTheme.spacing.pageInset),
                        onEdit = onEdit,
                        onDraft = onDraft,
                    )
                }
            }
            displayed?.let { current ->
                // 改不了的项、缺必填值的项和核心标出的问题挡在前面；保存时核心还会再校验一次。
                val blocked = fieldProblems.isNotEmpty() || current.form.fields.any { field ->
                    isUnsupported(field, edits) || isMissingRequired(field, edits) || field.problem != null
                }
                SettingsSaveArea(saving, blocked) { onSave(current, edits) }
            }
        }
    }
}

/** 保存；被拒时把核心按字段给的明细带回，别的失败预检一次拿最新视图，也拿不到就重读整页。 */
private suspend fun saveSettings(
    plugins: PluginsState,
    id: String,
    view: PluginSettingsView,
    edits: Map<String, SettingValue?>,
): SaveResult = try {
    val updated = plugins.core.setPluginSettings(id, PluginSettingsUpdate(view.schemaDigest, edits))
    SaveResult(updated, willRestart = willRestart(updated, edits))
} catch (failure: CoreException) {
    if (failure is CoreException.SettingsInvalid) {
        // 校验没过：明细逐字段标到控件旁，不用再问核心。
        SaveResult(
            problem = failure.problem,
            fieldProblems = failure.fields.associate {
                it.name to SettingsFieldProblem(it.kind, it.message)
            },
        )
    } else {
        try {
            SaveResult(
                plugins.core.previewPluginSettings(id, PluginSettingsUpdate(view.schemaDigest, edits)),
                problem = failure.problem,
            )
        } catch (_: CoreException) {
            SaveResult(reload = true, problem = failure.problem)
        }
    }
}

/** 这次改动会不会让应用重启：改动的项里有服务用到（核心只重建这些服务）。 */
private fun willRestart(updated: PluginSettingsView, edits: Map<String, SettingValue?>): Boolean =
    updated.form.fields.any { it.name in edits && it.services.isNotEmpty() }

/** 表单正文：一项一节，有问题的项在控件旁标红；保存成功后提示会不会重启。 */
@Composable
internal fun SettingsForm(
    view: PluginSettingsView,
    edits: Map<String, SettingValue?>,
    drafts: Map<String, String>,
    failure: Problem?,
    restartHint: Boolean,
    fieldProblems: Map<String, SettingsFieldProblem> = emptyMap(),
    modifier: Modifier = Modifier,
    onEdit: (name: String, value: SettingValue?) -> Unit,
    onDraft: (field: PluginSettingField, draft: String) -> Unit,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        if (restartHint) {
            WontonNotice(WontonIcon.Info, stringResource(R.string.plugin_settings_restart_hint), isError = false)
        }
        failure?.let { ProblemNotice(it) }
        if (view.form.fields.any { isUnsupported(it, edits) }) {
            WontonNotice(WontonIcon.Warning, stringResource(R.string.plugin_settings_update_app), isError = false)
        }
        view.form.fields.forEach { field ->
            SettingsField(
                field, effectiveValue(field, edits), textDraft(field, edits, drafts),
                problem = fieldProblems[field.name] ?: field.problem,
                disabled = isUnsupported(field, edits),
                missing = isMissingRequired(field, edits),
                onEdit = onEdit,
                onDraft = onDraft,
            )
        }
    }
}

/** 底部的保存：进行中换成转圈（跟其他页一致），没法保存时按钮置灰。 */
@Composable
internal fun SettingsSaveArea(saving: Boolean, blocked: Boolean, onSave: () -> Unit) {
    if (saving) {
        Box(
            Modifier.fillMaxWidth().padding(bottom = LocalWontonBottomInset.current)
                .padding(WontonTheme.spacing.pageInset),
            contentAlignment = Alignment.Center,
        ) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = 2.dp)
        }
    } else {
        WontonButton(
            stringResource(R.string.common_save),
            onSave,
            Modifier.padding(bottom = LocalWontonBottomInset.current)
                .padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
            enabled = !blocked,
        )
    }
}
