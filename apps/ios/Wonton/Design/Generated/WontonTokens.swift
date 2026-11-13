// 由 scripts/generate-design.py 从 design/ 生成，不要手改。
import SwiftUI

enum WontonSpacing {
    static let s4: CGFloat = 4
    static let s8: CGFloat = 8
    static let s12: CGFloat = 12
    static let s16: CGFloat = 16
    static let s24: CGFloat = 24
    static let s32: CGFloat = 32
    static let s48: CGFloat = 48
    static let pageInset: CGFloat = 16
    static let sectionGap: CGFloat = 24
}

enum WontonRadii {
    static let control: CGFloat = 14
    static let card: CGFloat = 20
    static let bubble: CGFloat = 18
    static let bubbleJoined: CGFloat = 6
    static let pill: CGFloat = 999
    static let bar: CGFloat = 999
    static let barItem: CGFloat = 999
    static let composerField: CGFloat = 20
}

enum WontonSize {
    static let target: CGFloat = 48
    static let avatar: CGFloat = 48
    static let topBar: CGFloat = 56
    static let bottomBar: CGFloat = 64
    static let icon: CGFloat = 24
    static let iconSmall: CGFloat = 20
    static let iconTiny: CGFloat = 14
    static let sendButton: CGFloat = 36
    static let composerField: CGFloat = 40
    static let fileThumbnail: CGFloat = 44
    static let menuBlur: CGFloat = 12
    static let menuWidth: CGFloat = 200
    static let menuElevation: CGFloat = 8
    static let messageImage: CGFloat = 240
    static let launcherIcon: CGFloat = 62
    static let launcherCell: CGFloat = 76
    static let storeIcon: CGFloat = 56
    static let taskIcon: CGFloat = 40
    static let floatingActionButton: CGFloat = 56
    static let previewIcon: CGFloat = 64
    static let detailIcon: CGFloat = 112
    static let screenshot: CGFloat = 380
    static let chip: CGFloat = 36
    static let smallButton: CGFloat = 32
    static let switchWidth: CGFloat = 44
    static let switchHeight: CGFloat = 26
    static let switchKnob: CGFloat = 22
    static let dialogWidth: CGFloat = 320
    static let grabberWidth: CGFloat = 36
    static let grabberHeight: CGFloat = 4
    static let hairline: CGFloat = 1
    static let quoteBar: CGFloat = 2
}

enum WontonRatio {
    static let pluginIconRadius: CGFloat = 0.2237
    static let secondaryContent: CGFloat = 0.7
    static let scrim: CGFloat = 0.4
    static let codeFill: CGFloat = 0.1
}

enum WontonGesture {
    static let pullDistance: CGFloat = 96
    static let pushDistance: CGFloat = 64
    static let swipeDistance: CGFloat = 64
    static let flickDistance: CGFloat = 32
    static let flickVelocity: CGFloat = 900
    static let dragSlop: CGFloat = 8
    static let edgeWidth: CGFloat = 24
}

enum WontonMotion {
    static let feedback: Double = 0.12
    static let state: Double = 0.18
    static let panel: Double = 0.24
    static let messageInsert: Double = 0.18
    static let reduced: Double = 0.1
}

extension WontonTextStyle {
    static let screenTitle = WontonTextStyle(size: 28, line: 34, weight: .bold, relativeTo: .title)
    static let sectionTitle = WontonTextStyle(size: 20, line: 26, weight: .semibold, relativeTo: .title3)
    static let itemTitle = WontonTextStyle(size: 17, line: 24, weight: .medium, relativeTo: .headline)
    static let body = WontonTextStyle(size: 16, line: 22, weight: .regular, relativeTo: .body)
    static let supporting = WontonTextStyle(size: 14, line: 20, weight: .regular, relativeTo: .subheadline)
    static let caption = WontonTextStyle(size: 12, line: 16, weight: .regular, relativeTo: .caption)
}
