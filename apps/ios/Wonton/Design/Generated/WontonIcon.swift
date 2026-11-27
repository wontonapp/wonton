// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
enum WontonIcon: String {
    case add
    case agent
    case agentFill
    case apps
    case appsFill
    case attach
    case back
    case blocked
    case cameraOff
    case chat
    case chatFill
    case check
    case checkCircle
    case checkCircleFill
    case clock
    case close
    case code
    case codeFill
    case compose
    case connection
    case copy
    case cpu
    case cpuFill
    case device
    case disk
    case document
    case documentFill
    case download
    case downloadFill
    case error
    case errorFill
    case expand
    case eye
    case file
    case folder
    case folderFill
    case forward
    case globe
    case globeFill
    case help
    case home
    case homeFill
    case image
    case imageFill
    case imageBroken
    case info
    case leave
    case list
    case lock
    case lockFill
    case lockKey
    case more
    case music
    case musicFill
    case notify
    case pause
    case pauseFill
    case person
    case pin
    case pinFill
    case play
    case playFill
    case qrCode
    case refresh
    case reply
    case search
    case send
    case serialPort
    case server
    case settings
    case settingsFill
    case share
    case shield
    case shieldFill
    case star
    case starFill
    case store
    case tasks
    case tasksFill
    case trash
    case upload
    case usb
    case videoCall
    case videoCallFill
    case voiceCall
    case voiceCallFill
    case warning
    case warningFill
    case warningCircle
    case warningCircleFill
    case web
    case wrench
    case wrenchFill
}

extension WontonIcon {
    /// 资源名的前半截：彩色版是它加 `Color`，单色版加 `Mono`。
    var assetName: String {
        switch self {
        case .add: "add"
        case .agent: "agent"
        case .agentFill: "agent"
        case .apps: "apps"
        case .appsFill: "apps"
        case .attach: "attach"
        case .back: "back"
        case .blocked: "blocked"
        case .cameraOff: "cameraOff"
        case .chat: "chat"
        case .chatFill: "chat"
        case .check: "check"
        case .checkCircle: "checkCircle"
        case .checkCircleFill: "checkCircle"
        case .clock: "clock"
        case .close: "close"
        case .code: "code"
        case .codeFill: "code"
        case .compose: "compose"
        case .connection: "connection"
        case .copy: "copy"
        case .cpu: "cpu"
        case .cpuFill: "cpu"
        case .device: "device"
        case .disk: "disk"
        case .document: "document"
        case .documentFill: "document"
        case .download: "download"
        case .downloadFill: "download"
        case .error: "error"
        case .errorFill: "error"
        case .expand: "expand"
        case .eye: "eye"
        case .file: "file"
        case .folder: "folder"
        case .folderFill: "folder"
        case .forward: "forward"
        case .globe: "globe"
        case .globeFill: "globe"
        case .help: "help"
        case .home: "home"
        case .homeFill: "home"
        case .image: "image"
        case .imageFill: "image"
        case .imageBroken: "imageBroken"
        case .info: "info"
        case .leave: "leave"
        case .list: "list"
        case .lock: "lock"
        case .lockFill: "lock"
        case .lockKey: "lockKey"
        case .more: "more"
        case .music: "music"
        case .musicFill: "music"
        case .notify: "notify"
        case .pause: "pause"
        case .pauseFill: "pause"
        case .person: "person"
        case .pin: "pin"
        case .pinFill: "pin"
        case .play: "play"
        case .playFill: "play"
        case .qrCode: "qrCode"
        case .refresh: "refresh"
        case .reply: "reply"
        case .search: "search"
        case .send: "send"
        case .serialPort: "serialPort"
        case .server: "server"
        case .settings: "settings"
        case .settingsFill: "settings"
        case .share: "share"
        case .shield: "shield"
        case .shieldFill: "shield"
        case .star: "star"
        case .starFill: "star"
        case .store: "store"
        case .tasks: "tasks"
        case .tasksFill: "tasks"
        case .trash: "trash"
        case .upload: "upload"
        case .usb: "usb"
        case .videoCall: "videoCall"
        case .videoCallFill: "videoCall"
        case .voiceCall: "voiceCall"
        case .voiceCallFill: "voiceCall"
        case .warning: "warning"
        case .warningFill: "warning"
        case .warningCircle: "warningCircle"
        case .warningCircleFill: "warningCircle"
        case .web: "web"
        case .wrench: "wrench"
        case .wrenchFill: "wrench"
        }
    }
}
