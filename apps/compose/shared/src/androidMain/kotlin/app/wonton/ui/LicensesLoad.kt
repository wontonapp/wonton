package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import app.wonton.client.ClientCore
import app.wonton.protocol.LicensesViewModel

/**
 * 许可清单的加载状态：还没读完是 `null`，读完是成功或失败。失败留在页面上让人重试，不静默当成空清单。
 *
 * 清单是构建时生成的资源文件，读它和解析它都在核心，壳只管把资源交过去。
 */
class LicensesLoad(
    val result: Result<LicensesViewModel>?,
    val retry: () -> Unit,
)

@Composable
internal fun rememberLicenses(core: ClientCore): LicensesLoad {
    val assets = LocalContext.current.assets
    var result by remember(core) { mutableStateOf<Result<LicensesViewModel>?>(null) }
    var attempt by remember(core) { mutableStateOf(0) }
    LaunchedEffect(core, attempt) {
        result = runCatching {
            val json = assets.open(LICENSES_ASSET).bufferedReader().use { it.readText() }
            core.licenses(json)
        }
    }
    return LicensesLoad(result) { attempt += 1 }
}

internal const val LICENSES_ASSET = "licenses.json"
