import Foundation

/// 相册界面的文字。表名跟着键分到的文件走（`scripts/ui_text/groups.py`），取字都经这里，页面里不写表名。
enum GalleryText {
    static var title: String { String(localized: .Photo4.photoTitle) }
    static var sectionTimeline: String { String(localized: .Photo3.photoSectionTimeline) }
    static var sectionAlbums: String { String(localized: .Photo5.photoSectionAlbums) }
    static var sectionSearch: String { String(localized: .Photo1.photoSectionSearch) }
    static var select: String { String(localized: .Photo1.photoSelect) }
    static func selectedCount(_ count: Int) -> String {
        String(localized: .Photo1.photoSelectedCount(count: count))
    }
    static var newAlbum: String { String(localized: .Photo5.photoNewAlbum) }
    static var albumName: String { String(localized: .Photo4.photoAlbumName) }
    static var create: String { String(localized: .Photo0.photoCreate) }
    static var albumsMine: String { String(localized: .Photo3.photoAlbumsMine) }
    static var albumsHousehold: String { String(localized: .Photo0.photoAlbumsHousehold) }
    static var albumsShared: String { String(localized: .Photo2.photoAlbumsShared) }
    static func itemCount(_ count: Int) -> String {
        String(localized: .Photo1.photoItemCount(count: count))
    }
    static func from(_ text: String) -> String {
        String(localized: .Photo1.photoFrom(text))
    }
    static var someoneElses: String { String(localized: .Photo1.photoSomeoneElses) }
    static var emptyTimeline: String { String(localized: .Photo1.photoEmptyTimeline) }
    static var emptyAlbums: String { String(localized: .Photo0.photoEmptyAlbums) }
    static var emptyAlbum: String { String(localized: .Photo1.photoEmptyAlbum) }
    static var emptySearch: String { String(localized: .Photo4.photoEmptySearch) }
    static var refresh: String { String(localized: .Photo0.photoRefresh) }
    static var searchPrompt: String { String(localized: .Photo3.photoSearchPrompt) }
    static var place: String { String(localized: .Photo4.photoPlace) }
    static var timeAny: String { String(localized: .Photo0.photoTimeAny) }
    static var timeMonth: String { String(localized: .Photo1.photoTimeMonth) }
    static var timeThisYear: String { String(localized: .Photo2.photoTimeThisYear) }
    static var timeLastYear: String { String(localized: .Photo0.photoTimeLastYear) }
    static var timeCustom: String { String(localized: .Photo2.photoTimeCustom) }
    static var timeFrom: String { String(localized: .Photo2.photoTimeFrom) }
    static var timeTo: String { String(localized: .Photo1.photoTimeTo) }
    static var kindPhoto: String { String(localized: .Photo1.photoKindPhoto) }
    static var kindVideo: String { String(localized: .Photo3.photoKindVideo) }
    static var favorites: String { String(localized: .Photo4.photoFavorites) }
    static var live: String { String(localized: .Photo0.photoLive) }
    static var actionFavorite: String { String(localized: .Photo4.photoActionFavorite) }
    static var actionUnfavorite: String { String(localized: .Photo1.photoActionUnfavorite) }
    static var actionAddToAlbum: String { String(localized: .Photo5.photoActionAddToAlbum) }
    static var actionRemoveFromAlbum: String { String(localized: .Photo2.photoActionRemoveFromAlbum) }
    static var actionShare: String { String(localized: .Photo4.photoActionShare) }
    static var actionTrash: String { String(localized: .Photo5.photoActionTrash) }
    static var actionInfo: String { String(localized: .Photo4.photoActionInfo) }
    static var undo: String { String(localized: .Photo5.photoUndo) }
    static func trashConfirm(_ count: Int) -> String {
        String(localized: .Photo2.photoTrashConfirm(count: count))
    }
    static var trashConfirmHint: String { String(localized: .Photo3.photoTrashConfirmHint) }
    static func doneFavorite(_ count: Int) -> String {
        String(localized: .Photo0.photoDoneFavorite(count: count))
    }
    static func doneUnfavorite(_ count: Int) -> String {
        String(localized: .Photo5.photoDoneUnfavorite(count: count))
    }
    static func doneTrash(_ count: Int) -> String {
        String(localized: .Photo3.photoDoneTrash(count: count))
    }
    static func doneRestore(_ count: Int) -> String {
        String(localized: .Photo4.photoDoneRestore(count: count))
    }
    static func doneAdd(_ count: Int) -> String {
        String(localized: .Photo0.photoDoneAdd(count: count))
    }
    static func doneRemove(_ count: Int) -> String {
        String(localized: .Photo2.photoDoneRemove(count: count))
    }
    static var doneCreate: String { String(localized: .Photo3.photoDoneCreate) }
    static var doneShare: String { String(localized: .Photo0.photoDoneShare) }
    static func partFailed(_ count: Int) -> String {
        String(localized: .Photo3.photoPartFailed(count: count))
    }
    static var failedEdit: String { String(localized: .Photo4.photoFailedEdit) }
    static var problemPluginTitle: String { String(localized: .Photo5.photoProblemPluginTitle) }
    static var problemPluginNext: String { String(localized: .Photo1.photoProblemPluginNext) }
    static var openStore: String { String(localized: .Photo4.photoOpenStore) }
    static var problemDeniedTitle: String { String(localized: .Photo4.photoProblemDeniedTitle) }
    static var problemDeniedNext: String { String(localized: .Photo4.photoProblemDeniedNext) }
    static var problemAuthTitle: String { String(localized: .Photo1.photoProblemAuthTitle) }
    static var problemBusyTitle: String { String(localized: .Photo0.photoProblemBusyTitle) }
    static var problemGoneTitle: String { String(localized: .Photo2.photoProblemGoneTitle) }
    static var problemUpstreamTitle: String { String(localized: .Photo4.photoProblemUpstreamTitle) }
    static var problemGenericTitle: String { String(localized: .Photo0.photoProblemGenericTitle) }
    static var problemRetryNext: String { String(localized: .Photo3.photoProblemRetryNext) }
    static var sourceTitle: String { String(localized: .Photo2.photoSourceTitle) }
    static var sourceNext: String { String(localized: .Photo4.photoSourceNext) }
    static var sourceNotInstalled: String { String(localized: .Photo4.photoSourceNotInstalled) }
    static var sourceChoose: String { String(localized: .Photo2.photoSourceChoose) }
    static var webVersion: String { String(localized: .Photo0.photoWebVersion) }
    static var videoFailed: String { String(localized: .Photo4.photoVideoFailed) }
    static var liveFailed: String { String(localized: .Photo1.photoLiveFailed) }
    static var imageFailed: String { String(localized: .Photo1.photoImageFailed) }
    static var play: String { String(localized: .Photo5.photoPlay) }
    static var infoTime: String { String(localized: .Photo1.photoInfoTime) }
    static var infoCamera: String { String(localized: .Photo5.photoInfoCamera) }
    static var infoPeople: String { String(localized: .Photo0.photoInfoPeople) }
    static var infoFile: String { String(localized: .Photo4.photoInfoFile) }
    static var listSeparator: String { String(localized: .Photo0.photoListSeparator) }
    static var shareUnavailable: String { String(localized: .Photo4.photoShareUnavailable) }
    static var shareDenied: String { String(localized: .Photo2.photoShareDenied) }
    static var shareEmpty: String { String(localized: .Photo3.photoShareEmpty) }
}
