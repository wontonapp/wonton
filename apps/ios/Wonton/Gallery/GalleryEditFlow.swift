import Observation
import SwiftUI
import WontonClient

/// 写操作从点下去到发给核心之间的那几步：挑相册、起名字、确认。发出去之后的进行中和结果都在核心的视图里
/// （`editing`、`lastEdit`）。本人点的、可撤的操作不问第二遍；这个源不接“恢复”时，进回收站先确认。
@MainActor
@Observable
final class GalleryEditFlow {
    /// 要加进相册的条目：开着“加进相册”的面板。
    var picking: [String]?
    /// 正在给新相册起名字；带着的条目建好就放进去，空表是只建相册。
    var naming: [String]?
    var albumName = ""
    /// 等确认进回收站的条目。
    var confirmingTrash: [String]?
    /// 正在挑共享给谁的相册。
    var sharing: PhotoAlbumRow?
    /// 面板收起之后才做的事：面板里选了“新建相册”要再弹起名字的框，两个呈现不能撞在一起。
    @ObservationIgnored private var afterPicking: (() -> Void)?
    private let model: GalleryModel

    init(model: GalleryModel) {
        self.model = model
    }

    private func edit(_ edit: PhotoEdit) {
        model.send(.edit(edit: edit))
    }

    func favorite(_ ids: [String], _ favorite: Bool) {
        edit(.favorite(ids: ids, favorite: favorite))
    }

    func trash(_ ids: [String]) {
        if model.supports(.restore) {
            edit(.trash(ids: ids))
        } else {
            confirmingTrash = ids
        }
    }

    func confirmTrash() {
        if let ids = confirmingTrash { edit(.trash(ids: ids)) }
        confirmingTrash = nil
    }

    func undoTrash(_ ids: [String]) {
        edit(.restore(ids: ids))
    }

    func add(_ ids: [String], to album: String) {
        edit(.addToAlbum(album: album, ids: ids))
    }

    func remove(_ ids: [String], from album: String) {
        edit(.removeFromAlbum(album: album, ids: ids))
    }

    /// 在“加进相册”的面板里选了一项：收起面板，收起之后再做。
    func pick(_ action: @escaping ([String]) -> Void) {
        let ids = picking ?? []
        afterPicking = { action(ids) }
        picking = nil
    }

    func finishPicking() {
        let action = afterPicking
        afterPicking = nil
        action?()
    }

    func startNaming(with ids: [String]) {
        albumName = ""
        naming = ids
    }

    func createAlbum() {
        let title = albumName.trimmingCharacters(in: .whitespacesAndNewlines)
        if let ids = naming, !title.isEmpty { edit(.createAlbum(title: title, ids: ids, shareWith: [])) }
        naming = nil
    }

    func share(_ album: PhotoAlbumRow, with people: [String]) {
        if !people.isEmpty { edit(.shareAlbum(album: album.id, people: people)) }
        sharing = nil
    }
}

/// 写操作的面板和确认框，挂在相册页的根上。
struct GalleryEditSheets: ViewModifier {
    @Bindable var flow: GalleryEditFlow
    let model: GalleryModel
    /// 这一页露在最上面：根页被相册页盖住时不由它来弹，两页抢同一个面板会弹不出来。
    var enabled = true

    private func shown(_ value: Bool) -> Bool { enabled && value }

    func body(content: Content) -> some View {
        content
            .sheet(
                isPresented: Binding(get: { shown(flow.picking != nil) }, set: { if !$0 { flow.picking = nil } }),
                onDismiss: flow.finishPicking
            ) {
                GalleryAlbumPicker(model: model) { choice in
                    switch choice {
                    case .new: flow.pick { flow.startNaming(with: $0) }
                    case let .album(id): flow.pick { flow.add($0, to: id) }
                    }
                }
            }
            .alert(
                GalleryText.newAlbum,
                isPresented: Binding(get: { shown(flow.naming != nil) }, set: { if !$0 { flow.naming = nil } })
            ) {
                TextField(GalleryText.albumName, text: $flow.albumName)
                Button(GalleryText.create) { flow.createAlbum() }
                Button(String(localized: .Common.commonCancel), role: .cancel) { flow.naming = nil }
            }
            .wontonDialog(
                GalleryText.trashConfirm(flow.confirmingTrash?.count ?? 0),
                message: GalleryText.trashConfirmHint,
                isPresented: Binding(
                    get: { shown(flow.confirmingTrash != nil) }, set: { if !$0 { flow.confirmingTrash = nil } }
                ),
                actions: [
                    WontonDialogAction(title: String(localized: .Common.commonCancel)),
                    WontonDialogAction(title: GalleryText.actionTrash, kind: .danger) { flow.confirmTrash() }
                ]
            )
            .sheet(
                isPresented: Binding(get: { shown(flow.sharing != nil) }, set: { if !$0 { flow.sharing = nil } })
            ) {
                if let album = flow.sharing {
                    GalleryShareSheet(album: album, model: model) { flow.share(album, with: $0) }
                }
            }
    }
}

/// “加进相册”的面板：新建一个，或者挑一个我的、家庭的相册。
private struct GalleryAlbumPicker: View {
    enum Choice {
        case new
        case album(String)
    }

    let model: GalleryModel
    let choose: (Choice) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text(GalleryText.actionAddToAlbum)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                    .padding(.horizontal, WontonTheme.spacing.pageInset)
                    .padding(.top, WontonTheme.spacing.s24)
                    .padding(.bottom, WontonTheme.spacing.s8)
                    .accessibilityAddTraits(.isHeader)
                if model.supports(.createAlbum) {
                    WontonSheetActionRow(action: WontonSheetAction(
                        title: GalleryText.newAlbum, icon: .add, perform: {}
                    )) { choose(.new) }
                }
                if model.supports(.addToAlbum) {
                    ForEach(model.view.albums.mine + model.view.albums.household, id: \.id) { album in
                        WontonSheetActionRow(action: WontonSheetAction(
                            title: album.title, icon: .folder, perform: {}
                        )) { choose(.album(album.id)) }
                    }
                }
            }
        }
        .background(Color(colors.canvas))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}

/// 把相册共享给家人：能选谁由核心给（`PhotosViewModel.shareTargets`），勾谁就共享给谁；名单取不到时照实说原因。
private struct GalleryShareSheet: View {
    let album: PhotoAlbumRow
    let model: GalleryModel
    let done: ([String]) -> Void
    @State private var picked: Set<String>
    @Environment(\.wontonColors) private var colors

    init(album: PhotoAlbumRow, model: GalleryModel, done: @escaping ([String]) -> Void) {
        self.album = album
        self.model = model
        self.done = done
        _picked = State(initialValue: Set(album.members.map(\.userId)))
    }

    private var targets: PhotoShareTargets { model.view.shareTargets }

    var body: some View {
        NavigationStack {
            Group {
                if let failure = targets.problem, targets.people.isEmpty {
                    unavailable(failure)
                } else if targets.people.isEmpty {
                    if targets.loading {
                        LoadingView()
                    } else {
                        GalleryEmptyView(icon: .person, title: GalleryText.shareEmpty) { EmptyView() }
                    }
                } else {
                    List(targets.people, id: \.userId) { person in
                        Button {
                            if picked.remove(person.userId) == nil { picked.insert(person.userId) }
                        } label: {
                            HStack {
                                Text(person.shownName).foregroundStyle(Color(colors.textPrimary))
                                Spacer(minLength: 0)
                                if picked.contains(person.userId) { WontonIcon.check.view() }
                            }
                        }
                        .accessibilityAddTraits(picked.contains(person.userId) ? .isSelected : [])
                    }
                }
            }
            .navigationTitle(GalleryText.actionShare)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: .Common.commonDone)) { done(Array(picked).sorted()) }
                        .disabled(targets.people.isEmpty)
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    /// 名单没取到：类别由核心给，没权限和别的原因分开说。
    private func unavailable(_ failure: WontonClient.MediaFailure) -> some View {
        let denied = failure.code == .permissionDenied
        let problem = Problem(
            title: GalleryText.shareUnavailable,
            nextStep: denied ? GalleryText.shareDenied : GalleryText.problemRetryNext,
            detail: failure.galleryProblem.detail
        )
        return ProblemView(problem: problem, retry: model.retry)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityIdentifier("gallery.share.unavailable")
    }
}

extension PhotoShareTarget {
    /// 没有显示名时写 Matrix ID 的本地部分（`@mom:example.com` 写成 `mom`），和核心给 `PhotoUser.name` 的写法一样。
    var shownName: String {
        if let displayName { return displayName }
        let local = userId.hasPrefix("@") ? userId.dropFirst() : Substring(userId)
        return local.split(separator: ":").first.map(String.init) ?? userId
    }
}
