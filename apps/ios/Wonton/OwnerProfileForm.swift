import Observation
import SwiftUI
import WontonClient

@MainActor
@Observable
final class OwnerProfileFormState {
    private(set) var profile: Me?
    private(set) var loadProblem: Problem?
    private(set) var saveProblem: Problem?
    private(set) var isSaving = false
    var name = ""
    var email = ""

    var canSave: Bool {
        profile != nil && !isSaving
            && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func load(core: ClientCore) async {
        profile = nil
        loadProblem = nil
        do {
            let profile = try await core.me()
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
            let saved = try await core.saveMe(update: OwnerProfileUpdate(displayName: name, email: email))
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
                OwnerProfileFields(name: $state.name, email: $state.email)
                if let problem = state.saveProblem {
                    ProblemNotice(problem: problem)
                }
                WontonGroup {
                    WontonRow(title: String(localized: .Peers.ownerProfileUsername), value: profile.username)
                }
            }
        } else if let problem = state.loadProblem {
            ProblemView(problem: problem, retry: retry)
        } else {
            LoadingView().frame(maxWidth: .infinity)
        }
    }
}

struct OwnerProfileFields: View {
    @Binding var name: String
    @Binding var email: String
    var nameProblem: String?
    var emailProblem: String?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        DetailSection(title: String(localized: .Peers.ownerProfileName)) {
            TextField("", text: $name)
                .accessibilityLabel(String(localized: .Peers.ownerProfileName))
                .textContentType(.name)
                .textFieldStyle(.plain)
                .wontonField()
            if let nameProblem { fieldProblem(nameProblem) }
        }
        DetailSection(title: String(localized: .Peers.ownerProfileEmail)) {
            TextField(String(localized: .Peers.ownerProfileEmailOptional), text: $email)
                .accessibilityLabel(String(localized: .Peers.ownerProfileEmail))
                .keyboardType(.emailAddress)
                .textContentType(.emailAddress)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.plain)
                .wontonField()
            if let emailProblem { fieldProblem(emailProblem) }
        }
    }

    private func fieldProblem(_ message: String) -> some View {
        Text(verbatim: message)
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.danger))
    }
}
