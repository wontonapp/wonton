import SwiftUI
import WontonClient

/// 你的资料（ADR-0030）：名字和邮箱存在家里的服务器上，要身份的应用从那里取。规则在家里的服务器上，不合规则的原因原样显示。
struct OwnerProfileView: View {
    let model: PluginsModel
    @Environment(\.dismiss) private var dismiss
    @State private var loaded: Result<OwnerProfile, Error>?
    @State private var name = ""
    @State private var email = ""
    @State private var problem: Problem?
    @State private var isSaving = false

    var body: some View {
        VStack(spacing: 0) {
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
            if case .success = loaded {
                Button(action: save) {
                    Text("保存").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(isSaving || trimmed(name).isEmpty || trimmed(email).isEmpty)
                .padding(WontonTheme.spacing.pageInset)
            }
        }
        .wontonModalScreen("你的资料")
        .task { await load() }
    }

    @ViewBuilder private var content: some View {
        switch loaded {
        case nil:
            LoadingView()
        case let .failure(error):
            ProblemView(problem: error.problem) {
                Task { await load() }
            }
        case let .success(profile):
            form(profile)
        }
    }

    private func form(_ profile: OwnerProfile) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                DetailSection(title: "名字") {
                    TextField("名字", text: $name)
                        .textContentType(.name)
                        .wontonField()
                }
                DetailSection(title: "邮箱") {
                    TextField("邮箱", text: $email, prompt: Text(verbatim: "me@example.com"))
                        .keyboardType(.emailAddress)
                        .textContentType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .wontonField()
                }
                if let problem {
                    ProblemNotice(problem: problem)
                }
                WontonGroup {
                    WontonRow(title: "用户名", value: profile.username)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
    }

    private func trimmed(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func load() async {
        loaded = nil
        do {
            let profile = try await model.core.ownerProfile()
            name = profile.displayName ?? ""
            email = profile.email ?? ""
            loaded = .success(profile)
        } catch {
            loaded = .failure(error)
        }
    }

    private func save() {
        problem = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                _ = try await model.core.saveOwnerProfile(
                    update: OwnerProfileUpdate(displayName: name, email: email)
                )
                dismiss()
            } catch {
                problem = error.problem
            }
        }
    }
}
