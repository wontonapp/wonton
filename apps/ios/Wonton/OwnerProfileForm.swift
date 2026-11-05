import Observation
import SwiftUI
import WontonClient

@MainActor
@Observable
final class OwnerProfileFormState {
    private(set) var profile: OwnerProfile?
    private(set) var loadProblem: Problem?
    private(set) var saveProblem: Problem?
    private(set) var isSaving = false
    var name = ""
    var email = ""

    var canSave: Bool {
        profile != nil && !isSaving
            && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !email.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func load(core: ClientCore) async {
        profile = nil
        loadProblem = nil
        do {
            let profile = try await core.ownerProfile()
            name = profile.displayName ?? ""
            email = profile.email ?? ""
            self.profile = profile
        } catch {
            loadProblem = error.problem
        }
    }

    func save(core: ClientCore) async -> Bool {
        guard !isSaving, profile != nil else { return false }
        saveProblem = nil
        isSaving = true
        defer { isSaving = false }
        do {
            let saved = try await core.saveOwnerProfile(update: OwnerProfileUpdate(displayName: name, email: email))
            guard !Task.isCancelled else { return false }
            profile = saved
            return true
        } catch {
            guard !Task.isCancelled else { return false }
            saveProblem = error.problem
            return false
        }
    }
}

struct OwnerProfileForm: View {
    @Bindable var state: OwnerProfileFormState
    let retry: () -> Void

    var body: some View {
        if let profile = state.profile {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                DetailSection(title: "名字") {
                    TextField("名字", text: $state.name)
                        .textContentType(.name)
                        .textFieldStyle(.plain)
                        .wontonField()
                }
                DetailSection(title: "邮箱") {
                    TextField("邮箱", text: $state.email, prompt: Text(verbatim: "me@example.com"))
                        .keyboardType(.emailAddress)
                        .textContentType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.plain)
                        .wontonField()
                }
                if let problem = state.saveProblem {
                    ProblemNotice(problem: problem)
                }
                WontonGroup {
                    WontonRow(title: "用户名", value: profile.username)
                }
            }
        } else if let problem = state.loadProblem {
            ProblemView(problem: problem, retry: retry)
        } else {
            LoadingView().frame(maxWidth: .infinity)
        }
    }
}
