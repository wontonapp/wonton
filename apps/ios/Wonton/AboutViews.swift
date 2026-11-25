import SwiftUI
import WontonClient

/// 许可清单的加载状态。清单是构建时生成的资源文件，读文件和解析都在核心，壳只管把文件交过去；
/// 读不出来留在页面上让人重试，不静默当成空清单。
@MainActor
@Observable
final class LicensesModel {
    enum State {
        case loading
        case failed(String?)
        case loaded([LicenseGroup])
    }

    private(set) var state = State.loading

    func load(core: ClientCore) async {
        do {
            guard ProcessInfo.processInfo.environment["WONTON_LICENSES_MISSING"] != "1",
                  let url = Bundle.main.url(forResource: "licenses", withExtension: "json") else {
                state = .failed(nil)
                return
            }
            let json = try String(contentsOf: url, encoding: .utf8)
            state = .loaded(try core.licenses(licensesJson: json).licenses)
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    func retry(core: ClientCore) {
        state = .loading
        Task { await load(core: core) }
    }

    func license(id: String) -> LicenseGroup? {
        if case let .loaded(licenses) = state { return licenses.first { $0.id == id } }
        return nil
    }

    var count: Int? {
        if case let .loaded(licenses) = state { return licenses.count }
        return nil
    }
}

struct AboutView: View {
    let licenses: LicensesModel
    let core: ClientCore
    @Environment(\.appVersionOverride) private var appVersionOverride

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                WontonGroup {
                    WontonRow(
                        title: String(localized: .Plugin2.pluginInfoVersion),
                        value: appVersionOverride ?? AppVersion.full
                    )
                }
                WontonGroup {
                    NavigationLink(value: SettingsRoute.privacy) {
                        WontonRow(title: String(localized: .Settings.settingsPrivacy), showsForward: true)
                    }
                    WontonSeparator()
                    NavigationLink(value: SettingsRoute.licenses) {
                        WontonRow(
                            title: String(localized: .Settings.settingsLicenses),
                            value: licenses.count.map { count in
                                String(localized: .Settings.settingsLicensesValue(count: count))
                            },
                            showsForward: true
                        )
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .buttonStyle(.plain)
        .wontonPushedScreen(String(localized: .Settings.settingsAbout), usesSystemToolbar: true)
        .task { await licenses.load(core: core) }
    }
}

struct PrivacyView: View {
    let core: ClientCore
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                ForEach(core.privacy().sections, id: \.title) { section in
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                        Text(section.title)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                        ForEach(section.paragraphs, id: \.self) { paragraph in
                            Text(paragraph)
                                .wontonText(WontonTheme.typography.body)
                        }
                    }
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(String(localized: .Settings.settingsPrivacy), usesSystemToolbar: true)
    }
}

struct LicensesView: View {
    let model: LicensesModel
    let core: ClientCore

    var body: some View {
        ScrollView {
            switch model.state {
            case .loading:
                EmptyView()
            case let .failed(reason):
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    Text(.Settings.settingsLicensesProblem).wontonText(WontonTheme.typography.body)
                    // 原因按类别由核心按 App 的语言给，收进默认收起的详情。
                    if let reason { ProblemDetail(text: reason, minimumTargetHeight: WontonTheme.size.target) }
                    Button(.Common.commonRetry) { model.retry(core: core) }
                        .buttonStyle(WontonSmallButtonStyle())
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(WontonTheme.spacing.pageInset)
            case let .loaded(licenses):
                LazyVStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                    ForEach(licenses, id: \.id) { license in
                        WontonGroup {
                            ForEach(Array(license.components.enumerated()), id: \.offset) { index, component in
                                if index > 0 { WontonSeparator() }
                                WontonRow(title: component.name, value: component.version)
                            }
                            WontonSeparator()
                            NavigationLink(value: SettingsRoute.licenseText(license.id)) {
                                WontonRow(
                                    title: String(localized: .Settings.settingsLicensesText),
                                    showsForward: true
                                )
                            }
                        }
                    }
                }
                .padding(WontonTheme.spacing.pageInset)
            }
        }
        .buttonStyle(.plain)
        .wontonPushedScreen(String(localized: .Settings.settingsLicenses), usesSystemToolbar: true)
        .task { await model.load(core: core) }
    }
}

struct LicenseTextView: View {
    let license: LicenseGroup

    var body: some View {
        ScrollView([.vertical, .horizontal]) {
            Text(license.text)
                .wontonText(WontonTheme.typography.caption)
                .fontDesign(.monospaced)
                .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen(license.name, usesSystemToolbar: true)
    }
}
