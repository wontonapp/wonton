import SwiftUI
import WontonClient

/// 外观设置：只渲染核心给出的选项，把选择发回核心。
struct AppearanceView: View {
    let model: AppearanceModel
    @Environment(\.wontonColors) private var colors
    @State private var showsMoreColors: Bool
    @State private var hexText: String
    @State private var isHexInvalid = false

    init(model: AppearanceModel) {
        self.model = model
        let appearance = model.viewModel
        _showsMoreColors = State(initialValue: !appearance.presets.contains(appearance.accent))
        _hexText = State(initialValue: accentHex(color: appearance.accent))
    }

    private var appearance: AppearanceViewModel { model.viewModel }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = model.rejection ?? appearance.problem?.problem {
                    ProblemNotice(problem: problem)
                }
                SettingsPreview()
                section(String(localized: .appearanceTheme)) {
                    choices(appearance.themeModes, title: \.label, selected: appearance.themeMode) {
                        model.send(.setThemeMode(mode: $0))
                    }
                }
                section(String(localized: .appearanceAccent)) { accent }
                section(String(localized: .appearanceTextSize)) {
                    choices(appearance.textSizes, title: \.label, selected: appearance.textSize) {
                        model.send(.setTextSize(size: $0))
                    }
                }
                section(String(localized: .appearanceFont)) { fonts }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .onChange(of: appearance.accent) { _, accent in
            hexText = accentHex(color: accent)
            isHexInvalid = false
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
                    model.send(.setFont(font: option))
                }
                .fontDesign(option.design)
            }
        }
    }

    private var accent: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            WontonGroup {
                swatches(appearance.presets, label: String(localized: .appearanceAccent))
                    .padding(WontonTheme.spacing.s8)
                WontonSeparator()
                Button {
                    showsMoreColors.toggle()
                } label: {
                    MoreColorsRow(
                        custom: appearance.presets.contains(appearance.accent)
                            ? nil : accentHex(color: appearance.accent),
                        isExpanded: showsMoreColors
                    )
                }
                .buttonStyle(.plain)
                if showsMoreColors {
                    WontonSeparator()
                    swatches(appearance.moreColors, label: String(localized: .appearanceMoreColors))
                        .padding(WontonTheme.spacing.s8)
                }
            }
            if showsMoreColors {
                hexField
            }
        }
    }

    /// 放在组外面：输入框的底和组的底同色，放进去就看不见了。
    @ViewBuilder private var hexField: some View {
        TextField("#RRGGBB", text: $hexText)
            .textFieldStyle(.plain)
            .keyboardType(.asciiCapable)
            .textInputAutocapitalization(.characters)
            .autocorrectionDisabled()
            .submitLabel(.done)
            .onSubmit(submitHex)
            .onChange(of: hexText) { isHexInvalid = false }
            .wontonField()
            .accessibilityLabel(String(localized: .appearanceColorValue))
        if isHexInvalid {
            Text(.appearanceHexInvalid)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.danger))
        }
    }

    private func swatches(_ swatches: [Rgb], label: String) -> some View {
        let columns = Array(repeating: GridItem(.flexible(), spacing: 0), count: Int(appearance.swatchColumns))
        return LazyVGrid(columns: columns, spacing: 0) {
            ForEach(Array(swatches.enumerated()), id: \.offset) { index, swatch in
                let isSelected = swatch == appearance.accent
                Button {
                    model.send(.setAccentColor(color: swatch))
                } label: {
                    Swatch(color: swatch, isSelected: isSelected)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(label) \(index + 1)")
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
        }
    }

    private func submitHex() {
        if let color = parseAccent(text: hexText) {
            model.send(.setAccentColor(color: color))
        } else {
            isHexInvalid = true
        }
    }
}

/// 设置里各个选项的名字，顺序和核心给的列表一致。
private extension ThemeMode {
    var label: String {
        switch self {
        case .system: String(localized: .appearanceSystem)
        case .light: String(localized: .appearanceThemeLight)
        case .dark: String(localized: .appearanceThemeDark)
        }
    }
}

private extension TextSize {
    var label: String {
        switch self {
        case .system: String(localized: .appearanceSystem)
        case .small: String(localized: .appearanceSizeSmall)
        case .standard: String(localized: .appearanceSizeStandard)
        case .large: String(localized: .appearanceSizeLarge)
        case .extraLarge: String(localized: .appearanceSizeExtraLarge)
        }
    }
}

private extension FontStyle {
    var label: String {
        switch self {
        case .default: String(localized: .appearanceFontDefault)
        case .rounded: String(localized: .appearanceFontRounded)
        case .serif: String(localized: .appearanceFontSerif)
        case .monospaced: String(localized: .appearanceFontMonospaced)
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
                        .foregroundStyle(Color(colors.accentInk))
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

private struct MoreColorsRow: View {
    /// 选的颜色不在色板里时的色值。
    let custom: String?
    let isExpanded: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            Text(.appearanceMore)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            Spacer(minLength: 0)
            if let custom {
                Text(custom)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
                .rotationEffect(.degrees(isExpanded ? 90 : 0))
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target)
        .contentShape(Rectangle())
        .accessibilityValue(isExpanded ? Text(.appearanceExpanded) : Text(.appearanceCollapsed))
    }
}

/// 一段小对话：改的每一项都在这里当场看到。
private struct SettingsPreview: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            HStack {
                Text(.appearancePreviewMessage)
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
                    Text(.appearancePreviewName).foregroundStyle(Color(colors.textOnAccentFill))
                    Text("21:08")
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.accentInkOnFill))
                }
                .padding(.horizontal, WontonTheme.spacing.s12)
                .padding(.vertical, WontonTheme.spacing.s8)
                .accentFill(RoundedRectangle(cornerRadius: WontonTheme.radii.bubble))
            }
        }
        .wontonText(WontonTheme.typography.body)
        .accessibilityHidden(true)
    }
}

/// 选中时外面多一圈：只靠颜色的话，和页面底色接近的色板看不出选没选中。
private struct Swatch: View {
    let color: Rgb
    let isSelected: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ZStack {
            Circle()
                .fill(Color(color))
                .overlay { Circle().strokeBorder(Color(colors.separator), lineWidth: 1) }
                .frame(width: 36, height: 36)
            if isSelected {
                Circle().strokeBorder(Color(colors.textPrimary), lineWidth: 2).frame(width: 46, height: 46)
            }
        }
        .frame(height: WontonTheme.size.target)
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
    }
}
