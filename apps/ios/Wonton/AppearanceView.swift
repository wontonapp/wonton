import SwiftUI
import WontonClient

/// 选重点色：预设、自定义 RGB 和恢复默认。颜色只由核心推导，这里只渲染。拖动滑块时只预览，松手才保存。
struct AppearanceView: View {
    let model: AppearanceModel
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.wontonColors) private var colors
    @State private var draft: Rgb
    @State private var hexText: String
    @State private var isHexInvalid = false

    init(model: AppearanceModel) {
        self.model = model
        _draft = State(initialValue: model.viewModel.accent)
        _hexText = State(initialValue: accentHex(color: model.viewModel.accent))
    }

    var body: some View {
        let preview = previewAccent(color: draft)
        let theme = colorScheme == .dark ? preview.dark : preview.light
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = model.rejection ?? model.viewModel.problem?.message {
                    Text(problem).wontonText(WontonTheme.typography.supporting).foregroundStyle(Color(colors.danger))
                }
                AppearancePreview(theme: theme)
                presets
                custom(ink: Color(theme.accentInk))
                Button {
                    model.send(.resetAccentColor)
                } label: {
                    Text("恢复默认").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .wontonPushedScreen("外观")
        .onChange(of: model.viewModel.accent) { _, accent in
            draft = accent
            hexText = accentHex(color: accent)
            isHexInvalid = false
        }
    }

    private var presets: some View {
        HStack(spacing: 0) {
            ForEach(Array(model.viewModel.presets.enumerated()), id: \.offset) { index, preset in
                if index > 0 { Spacer(minLength: 0) }
                let isSelected = preset == draft
                Button {
                    model.send(.setAccentColor(color: preset))
                } label: {
                    PresetSwatch(color: preset, isSelected: isSelected)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("重点色 \(index + 1)")
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
        }
    }

    private func custom(ink: Color) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            TextField("#RRGGBB", text: $hexText)
                .textFieldStyle(.plain)
                .keyboardType(.asciiCapable)
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .onSubmit(submitHex)
                .onChange(of: hexText) { isHexInvalid = false }
                .wontonField()
                .accessibilityLabel("颜色值")
            if isHexInvalid {
                Text("颜色要写成 #RRGGBB")
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
            }
            channel("R", spoken: "红", value: $draft.red, ink: ink)
            channel("G", spoken: "绿", value: $draft.green, ink: ink)
            channel("B", spoken: "蓝", value: $draft.blue, ink: ink)
        }
    }

    private func channel(_ name: String, spoken: String, value: Binding<UInt8>, ink: Color) -> some View {
        ChannelSlider(
            name: name,
            spokenName: spoken,
            value: value,
            ink: ink,
            onChange: { hexText = accentHex(color: draft) },
            onCommit: { model.send(.setAccentColor(color: draft)) }
        )
    }

    private func submitHex() {
        if let color = parseAccent(text: hexText) {
            model.send(.setAccentColor(color: color))
        } else {
            isHexInvalid = true
        }
    }
}

private struct AppearancePreview: View {
    let theme: ThemeColors

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text("今晚吃什么？")
                    .foregroundStyle(Color(theme.textPrimary))
                    .bubble(Color(theme.surfaceSubtle), border: nil)
                Spacer(minLength: 48)
            }
            HStack {
                Spacer(minLength: 48)
                HStack(alignment: .lastTextBaseline, spacing: 8) {
                    Text("云吞").foregroundStyle(Color(theme.textOnAccentFill))
                    Text("21:08")
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(theme.accentInkOnFill))
                }
                .bubble(Color(theme.accentFill), border: theme.accentFillBorder.map { Color($0) })
            }
        }
        .wontonText(WontonTheme.typography.body)
        .accessibilityHidden(true)
    }
}

private extension View {
    func bubble(_ fill: Color, border: Color?) -> some View {
        padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background {
                let shape = RoundedRectangle(cornerRadius: 16)
                shape.fill(fill).overlay {
                    if let border { shape.strokeBorder(border, lineWidth: 1) }
                }
            }
    }
}

/// 选中时外面多一圈：只靠颜色的话，和页面底色接近的预设看不出选没选中。
private struct PresetSwatch: View {
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
        .frame(width: 48, height: 48)
        .contentShape(Rectangle())
    }
}

/// SwiftUI 的 `Slider` 换不了轨道和滑块，两端要一样只能自己画。
private struct ChannelSlider: View {
    private static let thumb: CGFloat = 24

    let name: String
    let spokenName: String
    @Binding var value: UInt8
    let ink: Color
    let onChange: () -> Void
    let onCommit: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: 12) {
            Text(name).frame(width: 16, alignment: .leading)
            GeometryReader { geometry in
                let span = geometry.size.width - Self.thumb
                let position = span * CGFloat(value) / 255
                ZStack(alignment: .leading) {
                    Capsule().fill(Color(colors.surfaceSubtle)).frame(height: 4).padding(.horizontal, Self.thumb / 2)
                    Capsule().fill(ink).frame(width: position, height: 4).offset(x: Self.thumb / 2)
                    Circle().fill(ink).frame(width: Self.thumb, height: Self.thumb).offset(x: position)
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { drag in
                            let fraction = (drag.location.x - Self.thumb / 2) / span
                            value = UInt8((min(max(fraction, 0), 1) * 255).rounded())
                            onChange()
                        }
                        .onEnded { _ in onCommit() }
                )
            }
            .frame(height: 48)
            Text(String(value)).monospacedDigit().frame(width: 32, alignment: .trailing)
        }
        .wontonText(WontonTheme.typography.body)
        .foregroundStyle(Color(colors.textPrimary))
        .accessibilityElement()
        .accessibilityLabel(spokenName)
        .accessibilityValue(String(value))
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: value = UInt8(min(Int(value) + 5, 255))
            case .decrement: value = UInt8(max(Int(value) - 5, 0))
            @unknown default: return
            }
            onChange()
            onCommit()
        }
    }
}
