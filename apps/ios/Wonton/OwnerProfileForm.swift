import Observation
import PhotosUI
import SwiftUI
import WontonClient

@MainActor
@Observable
final class OwnerProfileFormState {
    private(set) var profile: Me?
    private(set) var loadProblem: Problem?
    private(set) var saveProblem: Problem?
    private(set) var avatarProblem: Problem?
    private(set) var isSaving = false
    private(set) var isAvatarBusy = false
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

    /// 相册选来的图先按规格裁成方形小图，再交给核心上传并写进资料。
    func setAvatar(_ item: PhotosPickerItem, core: ClientCore) async {
        guard !isAvatarBusy, profile != nil else { return }
        avatarProblem = nil
        isAvatarBusy = true
        defer { isAvatarBusy = false }
        do {
            let upload = try await AvatarTranscode.upload(from: item, slot: .profile, core: core) { _ in }
            guard !Task.isCancelled else { return }
            profile = try await core.setMyAvatar(upload: upload)
        } catch {
            guard !Task.isCancelled else { return }
            avatarProblem = (error as? AvatarTranscodeError)?.problem ?? error.problem
        }
    }

    func clearAvatar(core: ClientCore) async {
        guard !isAvatarBusy, profile != nil else { return }
        avatarProblem = nil
        isAvatarBusy = true
        defer { isAvatarBusy = false }
        do {
            profile = try await core.clearMyAvatar()
        } catch {
            guard !Task.isCancelled else { return }
            avatarProblem = error.problem
        }
    }
}

struct OwnerProfileForm: View {
    @Bindable var state: OwnerProfileFormState
    let core: ClientCore
    let retry: () -> Void
    @State private var showsPicker = false
    @State private var pickedItem: PhotosPickerItem?

    var body: some View {
        if let profile = state.profile {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                avatarSection(profile)
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

    private func avatarSection(_ profile: Me) -> some View {
        DetailSection(title: String(localized: .Peers.ownerProfileAvatar)) {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                avatar(profile)
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    Button(String(localized: .Peers.ownerProfileAvatarPick)) { showsPicker = true }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                        .disabled(state.isAvatarBusy)
                    if profile.avatar != nil {
                        Button(String(localized: .Peers.ownerProfileAvatarRemove), role: .destructive) {
                            Task { await state.clearAvatar(core: core) }
                        }
                        .buttonStyle(WontonButtonStyle(kind: .danger))
                        .disabled(state.isAvatarBusy)
                    }
                }
            }
            if let problem = state.avatarProblem {
                ProblemNotice(problem: problem) { state.clearAvatarProblem() }
            }
        }
        .photosPicker(isPresented: $showsPicker, selection: $pickedItem, matching: .images)
        .onChange(of: pickedItem) { _, item in
            guard let item else { return }
            pickedItem = nil
            Task { await state.setAvatar(item, core: core) }
        }
    }

    @ViewBuilder
    private func avatar(_ profile: Me) -> some View {
        ZStack {
            if let image = profile.avatar {
                AvatarMediaView(image: image, core: core, size: WontonTheme.size.detailIcon)
            } else {
                Circle().fill(Color.secondary.opacity(0.15))
                Text(verbatim: initial(profile))
                    .wontonText(WontonTheme.typography.screenTitle)
                    .foregroundStyle(.secondary)
            }
            if state.isAvatarBusy { ProgressView() }
        }
        .frame(width: WontonTheme.size.detailIcon, height: WontonTheme.size.detailIcon)
        .clipShape(Circle())
    }

    private func initial(_ profile: Me) -> String {
        let source = profile.displayName ?? profile.username
        return source.first.map(String.init) ?? ""
    }
}

extension OwnerProfileFormState {
    func clearAvatarProblem() {
        avatarProblem = nil
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
