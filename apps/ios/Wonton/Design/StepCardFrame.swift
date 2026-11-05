import SwiftUI
import WontonClient

struct StepCardFrame<Content: View>: View {
    let nav: StepNav
    let title: String
    let primaryTitle: String
    var isWorking = false
    let back: () -> Void
    let skip: () -> Void
    let next: () -> Void
    var rejection: (title: String, action: () -> Void)?
    @ViewBuilder let content: Content
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            navigation
            Text(verbatim: title)
                .wontonText(WontonTheme.typography.screenTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .accessibilityAddTraits(.isHeader)
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s16)
            content
            VStack(spacing: WontonTheme.spacing.s8) {
                Button(action: next) {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if isWorking { ProgressView().tint(Color(colors.onPrimary)) }
                        Text(primaryTitle)
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(!nav.canNext)
                if let rejection {
                    Button(rejection.title, action: rejection.action)
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    private var navigation: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            WontonIconButton(icon: .back, label: "上一步", action: back)
                .disabled(!nav.canBack)
                .opacity(nav.canBack ? 1 : WontonTheme.ratio.secondaryContent)
            Spacer(minLength: 0)
            Text("\(nav.position)/\(nav.total)")
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .accessibilityLabel("第 \(nav.position) 步，共 \(nav.total) 步")
            Spacer(minLength: 0)
            Button("跳过", action: skip)
                .buttonStyle(.plain)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .frame(minWidth: WontonTheme.size.target, minHeight: WontonTheme.size.target)
                .disabled(!nav.canSkip)
                .opacity(nav.canSkip ? 1 : WontonTheme.ratio.secondaryContent)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }
}
