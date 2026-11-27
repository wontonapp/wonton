import SwiftUI
import WontonClient

/// 网格里的一格：正方形裁切，角上标视频时长、实况、收藏和来自哪位家人；选择时左上角是勾。
struct GalleryTile: View {
    let tile: PhotoTile
    let scope: String
    let core: ClientCore
    /// 解码的像素边长，按列数算。
    let pixel: Int
    let selecting: Bool
    let selected: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Color.clear
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                GalleryImage(
                    request: GalleryImageRequest(
                        scope: scope, subject: .item(id: tile.id, grade: .thumbnail), maxPixel: pixel
                    ),
                    thumbhash: tile.thumbhash, core: core
                )
            }
            .clipped()
            // 压在照片上的小标记用单色版的图标，白字白图才读得清。
            .overlay(alignment: .bottom) { marks.environment(\.wontonIconMono, true) }
            .overlay(alignment: .topTrailing) { owner.environment(\.wontonIconMono, true) }
            .scaleEffect(selected ? 0.86 : 1)
            .background(Color(colors.surfaceSubtle))
            .overlay(alignment: .topLeading) { selectionMark }
            .contentShape(Rectangle())
            .animation(.easeOut(duration: WontonTheme.motion.feedback), value: selected)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(label)
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .accessibilityIdentifier("gallery.tile.\(tile.id)")
    }

    private var label: String {
        let kind = tile.isVideo ? GalleryText.kindVideo : GalleryText.kindPhoto
        return "\(kind), \(tile.takenDate.formatted(date: .abbreviated, time: .shortened))"
    }

    @ViewBuilder private var marks: some View {
        if tile.isVideo || tile.live || tile.favorite {
            HStack(spacing: WontonTheme.spacing.s4) {
                if tile.isVideo {
                    WontonIcon.playFill.view(size: WontonTheme.size.iconTiny)
                    if let duration = tile.durationMs {
                        Text(GalleryFormat.duration(milliseconds: duration)).monospacedDigit()
                    }
                } else if tile.live {
                    Text(GalleryText.live)
                }
                Spacer(minLength: 0)
                if tile.favorite { WontonIcon.starFill.view(size: WontonTheme.size.iconTiny) }
            }
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 6)
            .padding(.bottom, 5)
            .padding(.top, WontonTheme.spacing.s16)
            .background(
                LinearGradient(colors: [.clear, .black.opacity(0.5)], startPoint: .top, endPoint: .bottom)
            )
        }
    }

    @ViewBuilder private var owner: some View {
        if let name = tile.ownerBadge {
            HStack(spacing: 2) {
                WontonIcon.person.view(size: WontonTheme.size.iconTiny)
                Text(name).lineLimit(1)
            }
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(.white)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(.black.opacity(0.45), in: Capsule())
            .padding(WontonTheme.spacing.s4)
            .accessibilityLabel(tile.ownerLine ?? name)
        }
    }

    @ViewBuilder private var selectionMark: some View {
        if selecting {
            // 没选的是一个空圈，选上的是实底加勾：两种状态一眼分得开。
            ZStack {
                Circle().fill(selected ? Color(colors.primary) : Color.black.opacity(0.15))
                Circle().strokeBorder(.white, lineWidth: 1.5)
                if selected {
                    WontonIcon.check.view(size: WontonTheme.size.iconTiny)
                        .foregroundStyle(Color(colors.onPrimary))
                        .environment(\.wontonIconMono, true)
                }
            }
            .frame(width: 22, height: 22)
            .shadow(color: .black.opacity(0.35), radius: 2)
            .padding(6)
        }
    }
}

@MainActor
enum GalleryFormat {
    /// 视频时长：不到一小时写成 `分:秒`。
    static func duration(milliseconds: UInt64) -> String {
        let seconds = Int((Double(milliseconds) / 1000).rounded())
        let (hours, minutes, rest) = (seconds / 3600, seconds / 60 % 60, seconds % 60)
        if hours > 0 { return String(format: "%d:%02d:%02d", hours, minutes, rest) }
        return String(format: "%d:%02d", minutes, rest)
    }

    private static let dayParser: DateFormatter = {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter
    }()

    /// 核心给的 `YYYY-MM-DD` 写成给人看的日期：今年的不写年份。
    static func day(_ date: String) -> String {
        guard let parsed = dayParser.date(from: date) else { return date }
        let thisYear = Calendar.current.component(.year, from: parsed) == Calendar.current.component(.year, from: .now)
        let style = Date.FormatStyle.dateTime.month().day().weekday(.abbreviated)
        return parsed.formatted(thisYear ? style : style.year())
    }

    /// 核心给的 `YYYY-MM` 写成给人看的月份。
    static func month(_ month: String) -> String {
        guard let parsed = dayParser.date(from: month + "-01") else { return month }
        return parsed.formatted(.dateTime.year().month(.wide))
    }
}
