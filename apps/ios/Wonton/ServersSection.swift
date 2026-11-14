import SwiftUI
import WontonClient

struct ServersSection: View {
    let model: HomeModel
    /// 移除的确认框挂在整页上：挂在每一行上，几个全屏弹层抢同一个开关，哪个都弹不出来。
    @Binding var removing: ServerRow?
    @Environment(\.wontonColors) private var colors
    @State private var renaming: ServerRow?
    @State private var name = ""

    var body: some View {
        let renameID = renaming?.id
        WontonGroup {
            ForEach(model.servers.servers, id: \.id) { server in
                Button {
                    if !server.current { model.send(.home(action: .switchServer(id: server.id))) }
                } label: {
                    HStack(spacing: 0) {
                        WontonRow(title: server.name.localizedDisplayName)
                        if server.current {
                            Image(systemName: "checkmark")
                                .foregroundStyle(Color(colors.primary))
                                .padding(.trailing, WontonTheme.spacing.s16)
                                .accessibilityHidden(true)
                        }
                    }
                }
                .accessibilityAddTraits(server.current ? .isSelected : [])
                .contextMenu {
                    Button(.Common.commonRename, systemImage: "pencil") {
                        name = server.name.localizedDisplayName
                        renaming = server
                    }
                    Button(.Common.commonRemove, systemImage: "trash", role: .destructive) { removing = server }
                }
                WontonSeparator()
            }
            Button { model.send(.home(action: .addServer)) } label: {
                HStack(spacing: WontonTheme.spacing.s12) {
                    WontonIcon.add.view(size: WontonTheme.size.iconSmall)
                    Text(.Servers.serversAdd).wontonText(WontonTheme.typography.body)
                    Spacer(minLength: 0)
                }
                .foregroundStyle(Color(colors.textPrimary))
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.vertical, WontonTheme.spacing.s12)
                .frame(minHeight: WontonTheme.size.target)
                .contentShape(Rectangle())
            }
            WontonSeparator()
            NavigationLink(value: SettingsRoute.matrixLogin) { MatrixLoginEntryLabel() }
        }
        .buttonStyle(.plain)
        .alert(String(localized: .Common.commonRename), isPresented: Binding(
            get: { renaming != nil }, set: { if !$0 { renaming = nil } }
        )) {
            TextField(String(localized: .Servers.serversName), text: $name)
            Button(.Common.commonCancel, role: .cancel) { renaming = nil }
            Button(.Common.commonSave) {
                if let renameID { model.send(.home(action: .renameServer(id: renameID, name: name))) }
                renaming = nil
            }
            .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }
}

extension View {
    func removeServerDialog(_ removing: Binding<ServerRow?>, model: HomeModel) -> some View {
        let server = removing.wrappedValue
        return wontonDialog(
            String(localized: .Servers.serversRemoveTitle(server?.name.localizedDisplayName ?? "")),
            message: String(localized: .Servers.serversRemoveMessage),
            isPresented: Binding(
                get: { removing.wrappedValue != nil }, set: { if !$0 { removing.wrappedValue = nil } }
            ),
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)) { removing.wrappedValue = nil },
                WontonDialogAction(title: String(localized: .Common.commonRemove), kind: .danger) {
                    if let id = server?.id { model.send(.home(action: .removeServer(id: id))) }
                    removing.wrappedValue = nil
                }
            ]
        )
    }
}

extension ServerName {
    var localizedDisplayName: String {
        switch self {
        case let .named(name): name
        case .default(number: 1): String(localized: .Servers.serversDefaultFirst)
        case let .default(number): String(localized: .Servers.serversDefaultNumbered(Int(number)))
        }
    }
}
