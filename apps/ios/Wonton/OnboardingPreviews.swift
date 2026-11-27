#if DEBUG
import SwiftUI
import WontonClient

/// 重看引导的预览：每步左上的关闭，和确认页的三类汇总（改了的列新值，没改的标“不改动”）。假数据只在这里。
enum OnboardingPreviews {
    static func nav(position: UInt32) -> StepNav {
        StepNav(position: position, total: 5, canBack: position > 1, canSkip: false, canNext: true, canClose: true)
    }

    static var review: OnboardingReview {
        OnboardingReview(items: [
            item(id: "profile", kind: .profile, titles: ["Ada Lovelace"], detail: .unchanged),
            item(id: "assistant", kind: .assistant, titles: ["Wonton"], detail: nil),
            item(id: "model", kind: .model, titles: ["OpenRouter", "example/model-large"],
                 detail: .model(test: .ok)),
            item(id: "app.immich.photos", kind: .app, titles: [String(localized: .Plugins.previewAppName1)],
                 detail: nil,
                 permissions: [
                    PermissionLine(
                        permission: "storage.share.photos:read", text: "Read the shared folder photos",
                        risk: nil, notice: nil, supported: true, added: false, optional: false,
                        enabled: false, offLoss: nil, opened: nil
                    ),
                    PermissionLine(
                        permission: "devices.gpu", text: "Use the graphics card (high permission)",
                        risk: .high, notice: nil, supported: true, added: false, optional: false,
                        enabled: false, offLoss: nil, opened: nil
                    )
                 ],
                 notices: [
                    CardLine(text: "Wonton hasn't reviewed this app. Its program comes from Example.",
                             emphasis: true),
                    CardLine(text: "The programs aren't locked to a specific version: example/app:latest",
                             emphasis: false)
                 ])
        ], commit: .notStarted)
    }

    private static func item(
        id: String, kind: OnboardingReviewKind, titles: [String], detail: OnboardingReviewDetail?,
        permissions: [PermissionLine] = [], notices: [CardLine] = []
    ) -> OnboardingReviewItem {
        OnboardingReviewItem(
            id: id, kind: kind, titles: titles, detail: detail, icon: nil,
            permissions: permissions, notices: notices, state: .ready
        )
    }
}

struct OnboardingPreviewFrame<Content: View>: View {
    let step: UInt32
    let title: String
    let primaryTitle: String
    @ViewBuilder let content: Content

    var body: some View {
        StepCardFrame(
            nav: OnboardingPreviews.nav(position: step), title: title, primaryTitle: primaryTitle,
            back: {}, skip: {}, next: {}, close: {},
            content: {
                ScrollView {
                    content
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                }
            }
        )
    }
}

#Preview("Redo setup, first step") {
    OnboardingPreviewFrame(
        step: 1, title: String(localized: .Onboarding4.onboardingStepProfile),
        primaryTitle: String(localized: .Onboarding2.onboardingNext)
    ) { EmptyView() }
}

#Preview("Redo setup, later step") {
    OnboardingPreviewFrame(
        step: 3, title: String(localized: .Onboarding1.onboardingStepModel),
        primaryTitle: String(localized: .Onboarding2.onboardingNext)
    ) { EmptyView() }
}

#Preview("Redo setup, review") {
    // 行里没有图标时不取图，核心只为了构造 PluginsModel；数据目录用临时目录。
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("onboarding-preview")
    if let core = try? ClientCore(dataDir: directory.path) {
        OnboardingPreviewFrame(
            step: 5, title: String(localized: .Onboarding3.onboardingStepReview),
            primaryTitle: String(localized: .Onboarding3.onboardingDone)
        ) {
            OnboardingReviewStep(review: OnboardingPreviews.review, plugins: PluginsModel(core: core)) { _ in }
        }
    }
}

/// 确认页的模拟器截图入口：启动参数带 `wonton-onboarding-review-preview` 时渲染这一屏（同 `ProblemPreview` 的做法）。
enum OnboardingReviewPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-onboarding-review-preview")
    }

    @MainActor
    static func root(core: ClientCore) -> some View {
        OnboardingPreviewFrame(
            step: 5, title: String(localized: .Onboarding3.onboardingStepReview),
            primaryTitle: String(localized: .Onboarding3.onboardingDone)
        ) {
            OnboardingReviewStep(review: OnboardingPreviews.review, plugins: PluginsModel(core: core)) { _ in }
        }
    }
}
#endif
