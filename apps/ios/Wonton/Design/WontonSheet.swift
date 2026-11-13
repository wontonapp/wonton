import SwiftUI
import WontonClient

/// 底部面板里的一行操作。
struct WontonSheetAction {
    let title: String
    let icon: WontonIcon
    var isDanger = false
    let perform: () -> Void
}

extension View {
    /// 自己画的底部面板（ADR-0026）。往下拖、点空白处都能关。
    func wontonSheet(
        isPresented: Binding<Bool>,
        title: String? = nil,
        @ViewBuilder content: @escaping (_ close: @escaping (@escaping () -> Void) -> Void) -> some View
    ) -> some View {
        background {
            // 为什么这样挂，见 `wontonDialog`。
            Color.clear
                .fullScreenCover(isPresented: isPresented) {
                    WontonSheetView(title: title, finish: { isPresented.wrappedValue = false }, content: content)
                        .presentationBackground(.clear)
                }
                .transaction { $0.disablesAnimations = true }
        }
    }

    /// 列一组操作的底部面板：面板收起之后才执行，操作要弹别的页面时不会撞上。
    func wontonActionSheet(
        isPresented: Binding<Bool>,
        title: String? = nil,
        actions: [WontonSheetAction]
    ) -> some View {
        wontonSheet(isPresented: isPresented, title: title) { close in
            VStack(spacing: 0) {
                ForEach(actions.indices, id: \.self) { index in
                    WontonSheetActionRow(action: actions[index]) { close(actions[index].perform) }
                }
            }
        }
    }
}

/// 面板里的一行操作；自带表单的面板（如加好友）在表单下面接着用同样的行。
struct WontonSheetActionRow: View {
    let action: WontonSheetAction
    let choose: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: choose) {
            HStack(spacing: WontonTheme.spacing.s12) {
                action.icon.view()
                Text(action.title).wontonText(WontonTheme.typography.body)
                Spacer(minLength: 0)
            }
            .foregroundStyle(Color(action.isDanger ? colors.danger : colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct WontonSheetView<Content: View>: View {
    let title: String?
    let finish: () -> Void
    let content: (_ close: @escaping (@escaping () -> Void) -> Void) -> Content
    @State private var appeared = false
    @State private var dragged: CGFloat = 0
    @State private var panelHeight: CGFloat = 0
    @Environment(\.wontonColors) private var colors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack(alignment: .bottom) {
            Color.black
                .opacity(appeared ? WontonTheme.ratio.scrim : 0)
                .ignoresSafeArea()
                .onTapGesture { close {} }
                .accessibilityHidden(true)
            panel
                .opacity(reduceMotion && !appeared ? 0 : 1)
                .offset(y: appeared || reduceMotion ? dragged : panelHeight)
        }
        .accessibilityAction(.escape) { close {} }
        .onAppear {
            // 先量出面板多高，从屏幕外滑上来。
            DispatchQueue.main.async {
                withAnimation(.easeOut(duration: duration)) { appeared = true }
            }
        }
    }

    private var duration: Double {
        reduceMotion ? WontonTheme.motion.reduced : WontonTheme.motion.panel
    }

    private var panel: some View {
        VStack(spacing: 0) {
            Capsule()
                .fill(Color(colors.separator))
                .frame(width: WontonTheme.size.grabberWidth, height: WontonTheme.size.grabberHeight)
                .padding(.vertical, WontonTheme.spacing.s8)
                .accessibilityHidden(true)
            if let title {
                Text(title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.bottom, WontonTheme.spacing.s8)
                    .accessibilityAddTraits(.isHeader)
            }
            // 内容放得下时面板随内容高，放不下才滚动。
            ViewThatFits(in: .vertical) {
                content(close)
                ScrollView { content(close) }
            }
        }
        .padding(.bottom, WontonTheme.spacing.s8)
        .frame(maxWidth: .infinity)
        .background {
            UnevenRoundedRectangle(
                topLeadingRadius: WontonTheme.radii.card,
                topTrailingRadius: WontonTheme.radii.card
            )
            .fill(Color(colors.surface))
            .ignoresSafeArea(edges: .bottom)
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { panelHeight = $0 + 64 }
        .gesture(
            DragGesture()
                .onChanged { dragged = max(0, $0.translation.height) }
                .onEnded { value in
                    if value.predictedEndTranslation.height > panelHeight / 2 {
                        close {}
                    } else {
                        withAnimation(.easeOut(duration: duration)) { dragged = 0 }
                    }
                }
        )
    }

    private func close(then action: @escaping () -> Void) {
        withAnimation(.easeOut(duration: duration)) {
            appeared = false
            dragged = 0
        } completion: {
            finish()
            action()
        }
    }
}
