import SwiftUI
import WontonClient

struct InstallPermissions: View {
    let permissions: [PermissionLine]
    let notices: [CardLine]

    var body: some View {
        if !notices.isEmpty { NoticeLines(lines: notices) }
        if !permissions.isEmpty {
            DetailSection(title: "权限") {
                ForEach(permissions, id: \.permission) { PermissionRow(line: $0) }
            }
        }
    }
}
