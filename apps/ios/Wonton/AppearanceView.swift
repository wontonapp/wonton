import SwiftUI
import WontonClient

/// 外观设置：只渲染核心给出的选项，把选择发回核心。
struct AppearanceView: View {
    let model: AppearanceModel
    @Environment(\.wontonColors) private var colors

    private var appearance: AppearanceViewModel { model.viewModel }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = model.rejection ?? appearance.problem?.problem {
                    ProblemNotice(problem: problem)
                }
                SettingsPreview()
                section(String(localized: .Appearance1.appearanceTheme)) {
                    choices(appearance.themeModes, title: \.label, selected: appearance.themeMode) {
                        model.send(.settings(action: .setThemeMode(mode: $0)))
                    }
                }
                section(String(localized: .Appearance0.appearanceTextSize)) {
                    choices(appearance.textSizes, title: \.label, selected: appearance.textSize) {
                        model.send(.settings(action: .setTextSize(size: $0)))
                    }
                }
                section(String(localized: .Appearance0.appearanceFont)) { fonts }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(title)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .padding(.horizontal, WontonTheme.spacing.s16)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    private func choices<Value: Equatable>(
        _ options: [Value],
        title: KeyPath<Value, String>,
        selected: Value,
        select: @escaping (Value) -> Void
    ) -> some View {
        WontonGroup {
            ForEach(Array(options.enumerated()), id: \.offset) { index, option in
                if index > 0 { WontonSeparator() }
                ChoiceRow(title: option[keyPath: title], isSelected: option == selected) {
                    select(option)
                }
            }
        }
    }

    /// 每一行用自己的字体写，选之前就看得出差别。系统字体的圆体、衬线和等宽都不带中文字形，所以要有拉丁字母的样例。
    private var fonts: some View {
        WontonGroup {
            ForEach(Array(appearance.fonts.enumerated()), id: \.offset) { index, option in
                if index > 0 { WontonSeparator() }
                ChoiceRow(title: option.label, sample: "Aa", isSelected: option == appearance.font) {
                    model.send(.settings(action: .setFont(font: option)))
                }
                .fontDesign(option.design)
            }
        }
    }
}

/// 设置里各个选项的名字，顺序和核心给的列表一致。
private extension ThemeMode {
    var label: String {
        switch self {
        case .system: String(localized: .Appearance1.appearanceSystem)
        case .light: String(localized: .Appearance0.appearanceThemeLight)
        case .dark: String(localized: .Appearance0.appearanceThemeDark)
        }
    }
}

private extension TextSize {
    var label: String {
        switch self {
        case .system: String(localized: .Appearance1.appearanceSystem)
        case .small: String(localized: .Appearance1.appearanceSizeSmall)
        case .standard: String(localized: .Appearance0.appearanceSizeStandard)
        case .large: String(localized: .Appearance0.appearanceSizeLarge)
        case .extraLarge: String(localized: .Appearance0.appearanceSizeExtraLarge)
        }
    }
}

private extension FontStyle {
    var label: String {
        switch self {
        case .default: String(localized: .Appearance0.appearanceFontDefault)
        case .rounded: String(localized: .Appearance0.appearanceFontRounded)
        case .serif: String(localized: .Appearance0.appearanceFontSerif)
        case .monospaced: String(localized: .Appearance1.appearanceFontMonospaced)
        }
    }
}

private struct ChoiceRow: View {
    let title: String
    var sample: String?
    let isSelected: Bool
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s12) {
                Text(title)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if let sample {
                    Text(sample)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textSecondary))
                        .accessibilityHidden(true)
                }
                Spacer(minLength: 0)
                if isSelected {
                    WontonIcon.check.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textPrimary))
                }
            }
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

/// 一段小对话：改的每一项都在这里当场看到。
private struct SettingsPreview: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            HStack {
                Text(.Appearance1.appearancePreviewMessage)
                    .foregroundStyle(Color(colors.textPrimary))
                    .padding(.horizontal, WontonTheme.spacing.s12)
                    .padding(.vertical, WontonTheme.spacing.s8)
                    .background {
                        RoundedRectangle(cornerRadius: WontonTheme.radii.bubble).fill(Color(colors.surfaceSubtle))
                    }
                Spacer(minLength: WontonTheme.spacing.s48)
            }
            HStack {
                Spacer(minLength: WontonTheme.spacing.s48)
                HStack(alignment: .lastTextBaseline, spacing: WontonTheme.spacing.s8) {
                    Text(.Appearance1.appearancePreviewName).foregroundStyle(Color(colors.onPrimary))
                    Text("21:08")
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.onPrimary))
                }
                .padding(.horizontal, WontonTheme.spacing.s12)
                .padding(.vertical, WontonTheme.spacing.s8)
                .background {
                    RoundedRectangle(cornerRadius: WontonTheme.radii.bubble).fill(Color(colors.primary))
                }
            }
        }
        .wontonText(WontonTheme.typography.body)
        .accessibilityHidden(true)
    }
}
