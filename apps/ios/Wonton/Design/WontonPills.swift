import SwiftUI
import WontonClient

/// 搜索框：放大镜在前，有内容时能清空。
struct WontonSearchField: View {
    @Binding var text: String
    let prompt: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.search.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            TextField(prompt, text: $text)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .submitLabel(.search)
            if !text.isEmpty {
                Button { text = "" } label: {
                    WontonIcon.close.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("清除")
                .padding(.trailing, -WontonTheme.spacing.s12)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.target)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
    }
}

/// 筛选用的小胶囊：选中的用重点色。图形比按钮矮，命中区还是 48。
struct WontonChip: View {
    let title: String
    let isSelected: Bool
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            Text(title)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(isSelected ? colors.textOnAccentFill : colors.textPrimary))
                .padding(.horizontal, WontonTheme.spacing.s12)
                .frame(minHeight: WontonTheme.size.chip)
                .background {
                    if isSelected {
                        Color.clear.accentFill(Capsule())
                    } else {
                        Capsule().fill(Color(colors.surfaceSubtle))
                    }
                }
                .frame(minHeight: WontonTheme.size.target)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

/// 列表行尾的小按钮。图形比按钮矮，命中区还是 48。
struct WontonSmallButtonStyle: ButtonStyle {
    @Environment(\.wontonColors) private var colors
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.smallButton)
            .background(Capsule().fill(Color(colors.surfaceSubtle)))
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
            .opacity(isEnabled ? (configuration.isPressed ? 0.6 : 1) : 0.4)
            .animation(.easeOut(duration: WontonTheme.motion.feedback), value: configuration.isPressed)
    }
}
