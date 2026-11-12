import SwiftUI
import WontonClient

/// 弹窗里的一个按钮。
struct WontonDialogAction {
    let title: String
    var kind: WontonButtonStyle.Kind = .secondary
    var perform: () -> Void = {}
}

extension View {
    /// 自己画的弹窗（ADR-0026）。点哪个按钮都会关掉；点空白处关掉并走 `onDismiss`，不给 `actions` 时只有一个“好”，也走它。
    func wontonDialog(
        _ title: String,
        message: String? = nil,
        detail: String? = nil,
        isPresented: Binding<Bool>,
        actions: [WontonDialogAction]? = nil,
        onDismiss: (() -> Void)? = nil
    ) -> some View {
        background {
            // 挂在单独的空视图上：同一个视图上的弹出页互不抢，关掉系统转场也只影响这里。
            Color.clear
                .fullScreenCover(isPresented: isPresented) {
                    WontonDialogView(
                        title: title,
                        message: message,
                        detail: detail,
                        actions: actions ?? [
                            WontonDialogAction(title: String(localized: .designOk), perform: onDismiss ?? {})
                        ],
                        close: { isPresented.wrappedValue = false },
                        dismiss: onDismiss ?? {}
                    )
                    .presentationBackground(.clear)
                }
                .transaction { $0.disablesAnimations = true }
        }
    }
}

private struct WontonDialogView: View {
    let title: String
    let message: String?
    let detail: String?
    let actions: [WontonDialogAction]
    let close: () -> Void
    let dismiss: () -> Void
    @State private var appeared = false
    @Environment(\.wontonColors) private var colors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            Color.black
                .opacity(appeared ? WontonTheme.ratio.scrim : 0)
                .ignoresSafeArea()
                .onTapGesture(perform: closeWithoutChoosing)
                .accessibilityHidden(true)
            card
                .opacity(appeared ? 1 : 0)
                .scaleEffect(appeared || reduceMotion ? 1 : 0.96)
                .padding(WontonTheme.spacing.pageInset)
        }
        .accessibilityAction(.escape, closeWithoutChoosing)
        .onAppear {
            let duration = reduceMotion ? WontonTheme.motion.reduced : WontonTheme.motion.state
            withAnimation(.easeOut(duration: duration)) { appeared = true }
        }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            // 字号调大放不下时才滚动，平时弹窗随内容高。
            ViewThatFits(in: .vertical) {
                texts
                ScrollView { texts }
            }
            buttons
        }
        .padding(WontonTheme.spacing.s16)
        .frame(maxWidth: WontonTheme.size.dialogWidth)
        .background(RoundedRectangle(cornerRadius: WontonTheme.radii.card).fill(Color(colors.surface)))
    }

    private var texts: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(title)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .accessibilityAddTraits(.isHeader)
            if let message {
                Text(message)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            if let detail { ProblemDetail(text: detail) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private var buttons: some View {
        let stacked = VStack(spacing: WontonTheme.spacing.s8) { actionButtons }
        if actions.count == 2 {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: WontonTheme.spacing.s8) { actionButtons }
                stacked
            }
        } else {
            stacked
        }
    }

    private var actionButtons: some View {
        ForEach(actions.indices, id: \.self) { index in
            let action = actions[index]
            Button {
                close()
                action.perform()
            } label: {
                Text(action.title).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: action.kind))
        }
    }

    private func closeWithoutChoosing() {
        close()
        dismiss()
    }
}
