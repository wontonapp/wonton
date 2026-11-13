import SwiftUI
import WontonClient

struct WontonNativeRootBar<Trailing: View>: ViewModifier {
    let hasActions: Bool
    let trailing: Trailing
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.wontonOpenSidebar) private var openSidebar

    func body(content: Content) -> some View {
        content
            .navigationTitle(status.title ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(.visible, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    if let openSidebar {
                        WontonIconButton(
                            icon: .list, label: String(localized: .Sidebar.sidebarOpen), action: openSidebar
                        )
                        .environment(\.wontonSystemToolbar, true)
                    }
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if hasActions { trailing.environment(\.wontonSystemToolbar, true) }
                }
            }
            .safeAreaInset(edge: .top, spacing: 0) {
                if let line = status.line {
                    line
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .padding(.bottom, WontonTheme.spacing.s8)
                }
            }
    }
}

struct WontonNativePushedBar<Trailing: View>: ViewModifier {
    let title: String
    var icon: WontonIcon = .back
    var iconLabel = String(localized: .Design.designBack)
    var avatar: AnyView?
    var showsStatus = false
    var showsStatusLine = false
    var onClose: (() -> Void)?
    let trailing: Trailing
    @Environment(\.wontonBarStatus) private var status
    @Environment(\.wontonColors) private var colors
    @Environment(\.dismiss) private var dismiss

    private var line: AnyView? { showsStatusLine ? status.line : nil }

    func body(content: Content) -> some View {
        content
            .navigationTitle("")
            .navigationBarTitleDisplayMode(.inline)
            .navigationBarBackButtonHidden()
            .toolbar(.visible, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    WontonIconButton(icon: icon, label: iconLabel) {
                        if let onClose { onClose() } else { dismiss() }
                    }
                    .environment(\.wontonSystemToolbar, true)
                }
                ToolbarItem(placement: .principal) { heading }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    trailing.environment(\.wontonSystemToolbar, true)
                }
            }
            .safeAreaInset(edge: .top, spacing: 0) {
                if let line {
                    line
                        .padding(.horizontal, WontonTheme.spacing.pageInset)
                        .padding(.bottom, WontonTheme.spacing.s8)
                }
            }
    }

    private var heading: some View {
        VStack(spacing: 0) {
            HStack(spacing: WontonTheme.spacing.s8) {
                if let avatar { avatar }
                Text(title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
            }
            if showsStatus || showsStatusLine, line == nil, let subtitle = status.subtitle {
                Text(subtitle)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(1)
            }
        }
    }
}
