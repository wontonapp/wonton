import Foundation
import XCTest

/// 待发脚注的四种语言和 Android #721 的 `timeline_sending` 一一对应（#721 复审：只改显示层）。
/// 文案说的是“已经存在本机、还没送达”，不是“正在上传”；四种语言都钉住，改翻译时不会悄悄漏一种。
final class SendingLabelStringsTests: XCTestCase {
    func testSendingLabelMatchesAndroidInEveryLanguage() throws {
        let expected = [
            "en": "Saved, not delivered yet",
            "zh-Hans": "已保存，等待送达",
            "zh-Hant": "已儲存，等待送達",
            "ja": "保存済み、未配信",
        ]
        for (language, value) in expected {
            // String 的 locale: 只影响格式化、不挑 .lproj，得自己进这份语言的资源包查。
            let path = try XCTUnwrap(Bundle.main.path(forResource: language, ofType: "lproj"), "没有 \(language).lproj")
            let bundle = try XCTUnwrap(Bundle(path: path), "打不开 \(language).lproj")
            XCTAssertEqual(
                bundle.localizedString(forKey: "message.sending", value: nil, table: "message2"),
                value,
                "message.sending 的 \(language) 文案要和 Android timeline_sending 一致"
            )
        }
    }
}
