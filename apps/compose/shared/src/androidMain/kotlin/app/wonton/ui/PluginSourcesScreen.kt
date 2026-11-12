package app.wonton.ui

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts.OpenDocument
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.protocol.AddedSource
import app.wonton.protocol.CoreException
import app.wonton.protocol.SourceKind
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val BYTES_PER_MIB = 1024 * 1024

/** “添加来源”：填地址（家里的服务器判断是插件还是索引）、从文件安装、删除添加过的来源（ADR-0031）。 */
@Composable
fun AddSourceScreen(plugins: PluginsState, navigation: PluginNavigation) {
    val scope = rememberCoroutineScope()
    var address by remember { mutableStateOf("") }
    var adding by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<Problem?>(null) }
    val pickFile = rememberPluginArchivePicker(plugins, onProblem = { problem = Problem(it) }) { archive ->
        navigation.addingSource = false
        navigation.confirmation = PluginConfirmation.Upload(archive)
    }
    val close = { navigation.addingSource = false }
    val focus = LocalFocusManager.current
    WontonScreen(bar = { WontonPushedBar("添加来源", close, WontonIcon.Close, "关闭") }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            WontonNotice(WontonIcon.Warning, "这里添加的应用和文件都没有经过审核，只装你信任的。", isError = false)
            problem?.let { ProblemNotice(it) }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                SourceAddressField(address) { address = it }
                WontonButton(
                    "添加",
                    {
                        adding = true
                        problem = null
                        scope.launch {
                            try {
                                plugins.core.addStoreSource(address)
                                address = ""
                                focus.clearFocus()
                            } catch (failure: CoreException) {
                                problem = failure.problem
                            }
                            adding = false
                        }
                    },
                    Modifier.fillMaxWidth(),
                    enabled = !adding && address.isNotBlank(),
                )
            }
            WontonButton("从文件安装", pickFile, Modifier.fillMaxWidth(), WontonButtonKind.SECONDARY)
            val added = plugins.viewModel.store?.added.orEmpty()
            if (added.isNotEmpty()) {
                DetailSection("我添加的") {
                    WontonGroup {
                        added.forEachIndexed { index, source ->
                            if (index > 0) WontonSeparator()
                            AddedSourceRow(source) {
                                problem = null
                                scope.launch {
                                    try {
                                        plugins.core.removeStoreSource(source.address)
                                    } catch (failure: CoreException) {
                                        problem = failure.problem
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AddedSourceRow(source: AddedSource, onRemove: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().padding(
            horizontal = WontonTheme.spacing.s12,
            vertical = WontonTheme.spacing.s8,
        ),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                // 同一个主机上可以有好几个索引，只写主机分不清。
                source.name ?: if (source.kind == SourceKind.INDEX) source.address else source.provider,
                style = WontonTheme.typography.itemTitle,
                color = colors.textPrimary.toColor(),
            )
            if (source.name != null) {
                Text(
                    source.provider,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                )
            }
        }
        WontonIconButton(WontonIcon.Trash, "删除", onRemove)
    }
}

/** 换版本：填另一个来源，或者选文件。可以比装着的旧，确认页上会醒目提示（ADR-0031）。 */
@Composable
fun ChangeVersionScreen(plugins: PluginsState, navigation: PluginNavigation, id: String) {
    var address by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<Problem?>(null) }
    val pickFile = rememberPluginArchivePicker(plugins, onProblem = { problem = Problem(it) }) { archive ->
        navigation.changingVersion = null
        navigation.confirmation = PluginConfirmation.UpgradeUpload(id, archive)
    }
    val close = { navigation.changingVersion = null }
    WontonScreen(bar = { WontonPushedBar("换版本", close, WontonIcon.Close, "关闭") }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            problem?.let { ProblemNotice(it) }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                SourceAddressField(address) { address = it }
                WontonButton(
                    "继续",
                    {
                        navigation.changingVersion = null
                        navigation.confirmation = PluginConfirmation.UpgradeFrom(id, address)
                    },
                    Modifier.fillMaxWidth(),
                    enabled = address.isNotBlank(),
                )
            }
            WontonButton("从文件安装", pickFile, Modifier.fillMaxWidth(), WontonButtonKind.SECONDARY)
        }
    }
}

@Composable
private fun SourceAddressField(address: String, onChange: (String) -> Unit) {
    WontonTextField(
        address,
        onChange,
        Modifier.fillMaxWidth(),
        placeholder = "https://github.com/owner/repo",
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
    )
}

/** 选插件的压缩包并读出内容，返回打开选择器的动作。格式由家里的服务器按内容认，这里不按类型筛：`.tgz` 这类系统不一定认得。 */
@Composable
private fun rememberPluginArchivePicker(
    plugins: PluginsState,
    onProblem: (String) -> Unit,
    onPicked: (ByteArray) -> Unit,
): () -> Unit {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val launcher = rememberLauncherForActivityResult(OpenDocument()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            val limit = plugins.core.maxPluginArchiveBytes().toLong()
            readPluginArchive(context, uri, limit).fold(onPicked) { onProblem(it.message ?: it.toString()) }
        }
    }
    return { launcher.launch(arrayOf("*/*")) }
}

/** 太大的文件不读进内存，家里的服务器也不会收。 */
private suspend fun readPluginArchive(context: Context, uri: Uri, limit: Long): Result<ByteArray> =
    withContext(Dispatchers.IO) {
        try {
            val size = context.contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use {
                if (it.moveToFirst()) it.getLong(0) else null
            }
            if (size != null && size > limit) {
                return@withContext Result.failure(IOException("文件太大了，最多 ${limit / BYTES_PER_MIB} MiB"))
            }
            val bytes = context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                ?: return@withContext Result.failure(IOException("打不开这个文件"))
            Result.success(bytes)
        } catch (failure: IOException) {
            Result.failure(failure)
        }
    }
