package app.wonton.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import app.wonton.protocol.AgentModelsViewModel
import app.wonton.protocol.OnboardingStep
import app.wonton.ui.AgentModels
import app.wonton.ui.AgentModelsScreen
import app.wonton.ui.ModelBrowser
import app.wonton.ui.OnboardingAssistantStep
import app.wonton.ui.OnboardingModelStepCard
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/**
 * ADR-0129 的界面走查：用假的 ViewModel 把模型页、服务商页和引导的两步摆出来截图，不连服务器、点了没反应。
 * `-e screen <名字>` 选状态，名字见 [previewModels] 和下面的 `onboarding-*`。外观用这台设备上 App 自己的，跟系统明暗走。
 * 只在 debug 构建注册（src/debug/AndroidManifest.xml），不进 release。
 */
class ModelsPreviewActivity : ComponentActivity() {
    private var screen by mutableStateOf("")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        screen = intent?.getStringExtra(EXTRA_SCREEN).orEmpty()
        val core = (application as WontonApplication).core.getOrNull() ?: return
        setContent {
            WontonTheme(core.appearance()) {
                when (screen) {
                    SCREEN_ASSISTANT, SCREEN_ASSISTANT_PROBLEM -> AssistantPreview(screen == SCREEN_ASSISTANT_PROBLEM)
                    SCREEN_ONBOARDING_MODEL -> ModelStepPreview()
                    else -> AgentModelsScreen(fakeModels(previewModels(screen)), onBack = {})
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        screen = intent.getStringExtra(EXTRA_SCREEN).orEmpty()
    }

    @Composable
    private fun fakeModels(view: AgentModelsViewModel): AgentModels {
        val context = LocalContext.current
        val browser = remember(context) { ModelBrowser(context) {} }
        return AgentModels(PREVIEW_BOT, view, latest = { view }, send = {}, browser = browser)
    }

    @Composable
    private fun stepModifier() = Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())
        .statusBarsPadding().navigationBarsPadding().imePadding()

    @Composable
    private fun AssistantPreview(problem: Boolean) {
        val view = previewOnboarding(OnboardingStep.ASSISTANT, position = 2u, blocked = problem)
        OnboardingAssistantStep(view, requireNotNull(view.nav), send = {}, onClose = {}, modifier = stepModifier())
    }

    @Composable
    private fun ModelStepPreview() {
        val view = previewOnboarding(OnboardingStep.MODEL, position = 3u, blocked = true)
        OnboardingModelStepCard(
            view, requireNotNull(view.nav), fakeModels(previewModels(SCREEN_MODELS_NONE)),
            send = {}, onClose = {}, modifier = stepModifier(),
        )
    }
}

private const val EXTRA_SCREEN = "screen"
private const val SCREEN_ASSISTANT = "onboarding-assistant"
private const val SCREEN_ASSISTANT_PROBLEM = "onboarding-assistant-problem"
private const val SCREEN_ONBOARDING_MODEL = "onboarding-model"
