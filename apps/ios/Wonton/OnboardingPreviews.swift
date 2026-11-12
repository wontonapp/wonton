#if DEBUG
import SwiftUI
import WontonClient

/// 重看引导的预览：每步左上的关闭，和确认页的四行汇总（改了的列新值，没改的标“不变”）。假数据只在这里。
private enum OnboardingPreviews {
    static func nav(position: UInt32) -> StepNav {
        StepNav(position: position, total: 5, canBack: position > 1, canSkip: false, canNext: true, canClose: true)
    }

    static var review: OnboardingReview {
        OnboardingReview(items: [
            item(id: "profile", kind: .profile, titles: ["Ada Lovelace"], detail: .unchanged),
            item(id: "app.hermes.agent", kind: .agent, titles: ["Hermes"], detail: .setAsDefault),
            item(id: "model", kind: .model, titles: ["OpenAI"], detail: .unchanged),
            item(id: "app.immich.photos", kind: .app, titles: [String(localized: .Plugins.previewAppName1)],
                detail: nil)
        ], commit: .notStarted)
    }

    private static func item(
        id: String, kind: OnboardingReviewKind, titles: [String], detail: OnboardingReviewDetail?
    ) -> OnboardingReviewItem {
        OnboardingReviewItem(
            id: id, kind: kind, titles: titles, detail: detail, icon: nil,
            permissions: [], notices: [], state: .ready
        )
    }
}

private struct OnboardingPreviewFrame<Content: View>: View {
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
        step: 2, title: String(localized: .Onboarding1.onboardingStepAgent),
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
#endif
