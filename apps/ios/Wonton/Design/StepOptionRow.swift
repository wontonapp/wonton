import SwiftUI

struct StepOptionRow<Leading: View, Trailing: View>: View {
    let title: String
    var description: String?
    let isSelected: Bool
    var blocked: String?
    var neutralSelection = false
    let select: () -> Void
    @ViewBuilder var leading: Leading
    @ViewBuilder var trailing: Trailing
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            Button(action: select) {
                HStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
                    Group {
                        if isSelected {
                            SelectedMark(neutral: neutralSelection)
                        } else {
                            Circle()
                                .strokeBorder(lineWidth: WontonTheme.size.hairline)
                                .frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
                        }
                    }
                    .foregroundStyle(Color(isSelected && neutralSelection ? colors.primary :
                        isSelected ? colors.accentInkOnFill : colors.textSecondary))
                    leading
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(verbatim: title)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(isSelected && !neutralSelection ?
                                colors.textOnAccentFill : colors.textPrimary))
                        if let description {
                            Text(verbatim: description)
                                .wontonText(WontonTheme.typography.supporting)
                                .foregroundStyle(Color(colors.textSecondary))
                        }
                        if let blocked {
                            Text(verbatim: blocked)
                                .wontonText(WontonTheme.typography.supporting)
                                .foregroundStyle(Color(colors.danger))
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(minHeight: WontonTheme.size.target, alignment: .center)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(blocked != nil)
            .accessibilityAddTraits(isSelected ? .isSelected : [])
            trailing
        }
        .padding(WontonTheme.spacing.s12)
        .background {
            let shape = RoundedRectangle(cornerRadius: WontonTheme.radii.card)
            if isSelected && !neutralSelection {
                Color.clear.accentFill(shape)
            } else {
                shape.fill(Color(colors.surfaceSubtle))
            }
        }
    }
}

/// 选中的圈勾。用形状画，不用 `WontonIcon` 的 Image：iOS 27 会把行里的 Image 露成
/// 盖住整行的辅助功能元素，选中那一行的按钮就成了“点不到”（issue #793）。
private struct SelectedMark: View {
    let neutral: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ZStack {
            // 跟 checkCircleFill 图标同比例：24 点里圆占 19.5，勾 9 点、线宽 1.5。
            Circle()
                .fill(Color(neutral ? colors.primary : colors.accentInkOnFill))
                .padding(2)
            Checkmark()
                .stroke(Color(neutral ? colors.onPrimary : colors.accentFill),
                        style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
                .padding(7.5)
        }
        .frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
    }
}

private struct Checkmark: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.midY))
        path.addLine(to: CGPoint(x: rect.width * 0.4, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        return path
    }
}

extension StepOptionRow where Leading == EmptyView, Trailing == EmptyView {
    init(title: String, description: String? = nil, isSelected: Bool, blocked: String? = nil,
         neutralSelection: Bool = false, select: @escaping () -> Void) {
        self.init(title: title, description: description, isSelected: isSelected, blocked: blocked,
                  neutralSelection: neutralSelection, select: select,
                  leading: { EmptyView() }, trailing: { EmptyView() })
    }
}

extension StepOptionRow where Leading == EmptyView {
    init(title: String, description: String? = nil, isSelected: Bool, blocked: String? = nil,
         neutralSelection: Bool = false, select: @escaping () -> Void,
         @ViewBuilder trailing: @escaping () -> Trailing) {
        self.init(title: title, description: description, isSelected: isSelected, blocked: blocked,
                  neutralSelection: neutralSelection, select: select, leading: { EmptyView() }, trailing: trailing)
    }
}
