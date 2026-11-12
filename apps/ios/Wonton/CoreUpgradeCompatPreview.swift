#if DEBUG
import SwiftUI
import WontonClient

/// 核心升级页兼容块（ADR-0085）的预览：升级会新失效的应用、没查成的轻提示。
/// 假数据只在这里；图标画的是 PluginIconView 取不到图标时的占位，不连核心取图。
enum CoreUpgradeCompatPreviews {
    static var breaking: CoreUpgradeCompat {
        .breaking(plugins: [
            CoreUpgradeBreakingPlugin(
                id: "app.immich.photos",
                reasons: [CoreUpgradeBreakingReason(category: .specUnsupported, raw: "0.13")]
            ),
            CoreUpgradeBreakingPlugin(
                id: "app.copyparty.files",
                reasons: [CoreUpgradeBreakingReason(category: .unknownFeature, raw: "x-wonton.tabs")]
            )
        ])
    }

    static var apps: [CoreUpgradeCompatApp] {
        [
            CoreUpgradeCompatApp(id: "app.immich.photos", title: String(localized: .previewAppName1), icon: nil),
            CoreUpgradeCompatApp(
                id: "app.copyparty.files",
                title: String(localized: .previewAppName2),
                icon: nil
            )
        ]
    }
}

private struct PreviewAppIcon: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let shape = RoundedRectangle(
            cornerRadius: WontonTheme.size.taskIcon * WontonTheme.ratio.pluginIconRadius,
            style: .continuous
        )
        WontonIcon.appsFill.view(size: WontonTheme.size.taskIcon * 0.54)
            .foregroundStyle(Color(colors.accentInkOnFill))
            .frame(width: WontonTheme.size.taskIcon, height: WontonTheme.size.taskIcon)
            .background(shape.fill(Color(colors.accentFill)))
    }
}

#Preview("Apps that will break") {
    CoreUpgradeCompatSection(
        compat: CoreUpgradeCompatPreviews.breaking,
        apps: CoreUpgradeCompatPreviews.apps
    ) { _ in PreviewAppIcon() }
}

#Preview("Not checked") {
    CoreUpgradeCompatSection(
        compat: CoreUpgradeCompat.failed(problem: CoreUpgradeProblem(reason: .upstream, message: "")),
        icon: { _ in PreviewAppIcon() }
    )
}
#endif
