import SwiftUI

struct StepOptionRow<Leading: View, Trailing: View>: View {
    let title: String
    var description: String?
    let isSelected: Bool
    var blocked: String?
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
                            WontonIcon.checkCircleFill.view()
                        } else {
                            Circle()
                                .strokeBorder(lineWidth: WontonTheme.size.hairline)
                                .frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
                        }
                    }
                    .foregroundStyle(Color(isSelected ? colors.accentInkOnFill : colors.textSecondary))
                    leading
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(verbatim: title)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(isSelected ? colors.textOnAccentFill : colors.textPrimary))
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
            if isSelected {
                Color.clear.accentFill(shape)
            } else {
                shape.fill(Color(colors.surfaceSubtle))
            }
        }
    }
}

extension StepOptionRow where Leading == EmptyView, Trailing == EmptyView {
    init(title: String, description: String? = nil, isSelected: Bool, blocked: String? = nil,
         select: @escaping () -> Void) {
        self.init(title: title, description: description, isSelected: isSelected, blocked: blocked,
                  select: select, leading: { EmptyView() }) { EmptyView() }
    }
}

extension StepOptionRow where Leading == EmptyView {
    init(title: String, description: String? = nil, isSelected: Bool, blocked: String? = nil,
         select: @escaping () -> Void,
         @ViewBuilder trailing: @escaping () -> Trailing) {
        self.init(title: title, description: description, isSelected: isSelected, blocked: blocked,
                  select: select, leading: { EmptyView() }, trailing: trailing)
    }
}
