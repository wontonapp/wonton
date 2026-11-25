import Foundation

/// App 锁着时通知不露内容（ADR-0102 §4）：核心把锁定标记写进 App Group 的数据目录，扩展发通知前读它。
/// 标记文件是核心的 `app-lock-locked.json`：`{locked, locks_at_ms}`。宽限没走完时 `locked` 是 false、
/// `locks_at_ms` 是到点自己算锁的墙钟毫秒——iOS 后台进程几秒就挂起，核心的定时器不保证跑到；
/// 没在等锁时 `locks_at_ms` 是 `UInt64.max`。
///
/// 没写过（App 锁从没开过）算没锁；读不懂、缺字段、组容器拿不到（未签名的构建）算锁着，宁可少露内容。
enum PushLock {
    private static let fileName = "app-lock-locked.json"

    /// 标记文件的意思：已经锁上，或者宽限的截止时刻已到。
    static func locked(marker: Data?, now: UInt64 = nowMs()) -> Bool {
        guard let marker else { return false }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let value = try? decoder.decode(Marker.self, from: marker) else { return true }
        return value.locked || now >= value.locksAtMs
    }

    static func lockedNow() -> Bool {
        guard let directory = try? AppGroup.dataDirectory() else { return true }
        return locked(marker: try? Data(contentsOf: directory.appending(path: fileName)))
    }

    private static func nowMs() -> UInt64 {
        UInt64((Date.now.timeIntervalSince1970 * 1000).rounded())
    }

    private struct Marker: Decodable {
        let locked: Bool
        let locksAtMs: UInt64
    }
}
