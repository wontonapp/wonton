import XCTest

/// 聊天页照 Telegram 的几样交互（ADR-0109）：左滑回复、点引用跳到原消息、“向下”先回到跳之前那一条、
/// “/”命令菜单、消息下的选项按钮、顶栏的正在输入、群聊里组尾的头像。
/// 不要后端：用 `WONTON_UI_FAKE` 的假房间（核心的 `ui_fake_room.rs` 给条目，`ui_fake_timeline.rs` 响应操作）。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/ChatTelegramUITests
final class ChatTelegramUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let directRoom = "chatRoom.!ui-fake:home.example"
    private static let groupRoom = "chatRoom.!ui-fake-call:home.example"
    private static let notesRoom = "chatRoom.!ui-fake-notes:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
        // 顶栏叠放那条要翻遍整条时间线，默认 360 秒会被 kill 在半路、连断言里的树都拿不到。
        executionTimeAllowance = 900
        app = XCUIApplication()
        // 不恢复上一次的场景状态：房间记着上次看到哪一条（SceneStorage），上一条用例翻到别处会带到这一条来。
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        // 深色轮次由跑测脚本经 `TEST_RUNNER_WONTON_DARK=1` 传进来；XCUITest 每轮开场会把设备外观改回上一次的样子，
        // 只在外面 `simctl ui` 切会被盖回去。
        if ProcessInfo.processInfo.environment["WONTON_DARK"] == "1" {
            XCUIDevice.shared.appearance = .dark
        }
        UITestBackend.launch(app)
    }

    /// 向左拖一条消息，松手后输入框上方出现回复条。
    func testSwipeLeftStartsReply() {
        enter(Self.directRoom)
        XCTAssertFalse(app.buttons["replyCancel"].exists, "还没拖就在回复")
        let start = message(3).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -160, dy: 0)),
                    withVelocity: .default, thenHoldForDuration: 0.1)
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "左滑没进入回复\n\(app.debugDescription)")
        shot("reply-bar")
    }

    /// 点气泡里的引用跳到原消息；“向下”先回到跳之前那一条，再点一次才到底。
    func testQuoteJumpsToOriginalAndDownGoesBackFirst() {
        enter(Self.directRoom)
        let down = app.buttons["timelineDown"]
        let quote = app.buttons["messageQuote.ui-fake-21"]
        tap(down)
        reveal(quote)
        XCTAssertFalse(originalOnScreen(5, besides: quote), "还没跳，原消息就在屏上\n\(app.debugDescription)")
        quote.tap()
        XCTAssertTrue(wait(10) { self.originalOnScreen(5, besides: quote) }, "点引用没跳到原消息\n\(app.debugDescription)")
        shot("jump-highlight")
        XCTAssertFalse(inView(quote), "跳走以后被点的那条还在屏上")

        tap(down)
        XCTAssertTrue(wait(10) { self.inView(quote) }, "“向下”没先回到跳之前那一条\n\(app.debugDescription)")
        tap(down)
        let last = text(startingWith: "Fixture last line")
        XCTAssertTrue(wait(10) { last.exists && last.isHittable }, "返回栈空了以后“向下”没到底")
    }

    /// 跳转本身也是一路在滚：途中要出浮动日期；手一碰打断跳转，也要在原消息上闪一下点名（ADR-0109 第 4 节）。
    func testQuoteJumpShowsDateAndStillFlashesWhenTouched() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        let quote = app.buttons["messageQuote.ui-fake-21"]
        reveal(quote)
        let scroll = app.descendants(matching: .any)["roomScroll"]
        let before = flashCount(scroll)
        quote.tap()
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForExistence(timeout: 3), "跳的途中没有浮动日期\n\(app.debugDescription)")
        // 立刻碰一下列表：程序化滚动停下，闪光照样要点在原消息上。
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 20)),
                    withVelocity: .default, thenHoldForDuration: 0.05)
        // 闪光只亮一秒多，读得比它晚：按永远留着的计数和最后一行核对。
        XCTAssertTrue(wait(5) { flashCount(scroll) > before },
                      "手碰断跳转后原消息没闪\n\(app.debugDescription)")
        XCTAssertTrue(scroll.label.contains("last=ui-fake-5"), "闪的不是原消息\n\(app.debugDescription)")
    }

    /// 引用摘要按原文换行，两行都给，还是最多两行（和安卓一样）；输入框上方那条回复也是两行。
    func testQuoteSummaryKeepsBothLines() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        let quote = app.buttons["messageQuote.ui-fake-20"]
        reveal(quote)
        // 名字一行加摘要两行约 58；只剩一行约 38。
        XCTAssertGreaterThanOrEqual(quote.frame.height, 56, "引用摘要只剩一行\n\(app.debugDescription)")
        XCTAssertLessThanOrEqual(quote.frame.height, Self.quoteMaxHeight, "引用摘要超过两行")
        waitForFloatingDateToFade()
        shot("quote-two-lines")
    }

    /// 引用跳到那条“表情 + 一行字”的消息，左滑它：输入框上方的回复条也要两行。
    func testComposerReplySummaryKeepsBothLines() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        let quote = app.buttons["messageQuote.ui-fake-20"]
        reveal(quote)
        quote.tap()
        let original = app.descendants(matching: .any)["message.ui-fake-19"]
        XCTAssertTrue(wait(10) { self.inView(original) }, "跳转没到原消息\n\(app.debugDescription)")
        let start = original.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -160, dy: 0)),
                    withVelocity: .default, thenHoldForDuration: 0.1)
        let reply = app.descendants(matching: .any)["composerReplyQuote"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5), "左滑没进入回复\n\(app.debugDescription)")
        // 名字一行加摘要两行约 54；只剩一行约 38。
        XCTAssertGreaterThanOrEqual(reply.frame.height, 50, "输入框上的摘要只剩一行\n\(app.debugDescription)")
        shot("reply-two-lines")
    }

    /// 命令菜单占着右下角时“向下”让开，收起菜单再回来。
    func testDownButtonHidesWhileCommandMenuOpen() {
        enter(Self.directRoom)
        let down = app.buttons["timelineDown"]
        tap(down)
        dragDown()
        XCTAssertTrue(down.waitForExistence(timeout: 5), "离开底部后没有“向下”\n\(app.debugDescription)")
        let field = app.textViews["Message"]
        field.tap()
        field.typeText("/")
        XCTAssertTrue(app.buttons["command.help"].waitForExistence(timeout: 5), "敲“/”没开命令菜单")
        XCTAssertTrue(down.waitForNonExistence(timeout: 5), "菜单开着还压着“向下”\n\(app.debugDescription)")
        field.typeText(XCUIKeyboardKey.delete.rawValue)
        XCTAssertTrue(down.waitForExistence(timeout: 5), "收起菜单后“向下”没回来\n\(app.debugDescription)")
    }

    /// 顶栏下面照 Telegram 的叠放：插件横幅在最上，旁聊标题在它下面。
    func testPluginBannerSitsAboveSideChatTitle() {
        enter(Self.notesRoom)
        // 卡片挂在落点那条（`SIDE_CHAT_TOP_INDEX`）：等落点摆稳（落点滚动出的日期胶囊淡掉）它就在眼前；
        // 再不行往上翻一两段，不指望“向下”按钮，也不用横穿整条时间线。
        waitForFloatingDateToFade()
        // 卡片标识带根事件 ID（`sideChatCard.<root>`），按前缀取一张。
        let card = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "sideChatCard.")).firstMatch
        for _ in 0..<3 where !card.exists { dragDown() }
        XCTAssertTrue(card.waitForExistence(timeout: 10), "找不到旁聊卡\n\(app.debugDescription)")
        reveal(card)
        tap(card)
        // 盖着的那份主对话被 accessibilityHidden，横幅只留一份；先到先取仍稳妥。
        let banner = app.descendants(matching: .any)["pluginStatusBanner"].firstMatch
        let header = app.descendants(matching: .any)["sideChatHeader"].firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 5), "旁聊页顶栏下没有插件横幅\n\(app.debugDescription)")
        XCTAssertTrue(header.waitForExistence(timeout: 5), "旁聊页顶栏下没有旁聊标题\n\(app.debugDescription)")
        XCTAssertLessThan(banner.frame.maxY, header.frame.minY,
                          "插件横幅不在旁聊标题上面\n\(app.debugDescription)")
        waitForFloatingDateToFade()
        shot("side-chat-top")
        // 旁聊页自带一层导航栈：这是它那一层的根，左边缘右滑不把整个房间划走（ADR-0093）。
        let edge = app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
        edge.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.1)
        Thread.sleep(forTimeInterval: 0.6)
        XCTAssertTrue(header.exists, "右滑把房间划走了\n\(app.debugDescription)")
    }

    /// 选项按钮：点一个变成选中，另一个不能再点。
    func testChoiceButtonsAnswerOnce() {
        enter(Self.directRoom)
        let approve = app.buttons["messageChoice.✅"]
        let deny = app.buttons["messageChoice.❌"]
        tap(app.buttons["timelineDown"])
        reveal(approve)
        XCTAssertTrue(approve.isEnabled && deny.isEnabled, "还没选，按钮就点不了")
        XCTAssertFalse(approve.isSelected || deny.isSelected, "还没选，就有按钮是选中的")
        waitForFloatingDateToFade()
        shot("choices-open")
        approve.tap()
        XCTAssertTrue(wait(5) { approve.isSelected }, "点了的选项没变成选中")
        XCTAssertFalse(deny.isSelected, "没点的选项成了选中")
        XCTAssertFalse(deny.isEnabled, "选过以后另一个还能点")
        shot("choices-answered")
    }

    /// 输入框开头敲“/”弹出命令列表，按前缀过滤；点一条直接发出去，输入框里的草稿留着、列表收起；“菜单”按钮打开同一张列表。
    func testSlashOpensCommandMenuAndTapSends() {
        enter(Self.directRoom)
        let field = input
        let commands = ["help", "model", "new"].map { app.buttons["command.\($0)"] }
        XCTAssertFalse(commands[0].exists, "没敲“/”就有命令列表")
        focus(field)
        field.typeText("/")
        for command in commands {
            XCTAssertTrue(command.waitForExistence(timeout: 5), "敲“/”后列表里少了 \(command)")
        }
        shot("command-menu")
        field.typeText("m")
        XCTAssertTrue(commands[0].waitForNonExistence(timeout: 5), "按前缀过滤后不匹配的命令还在")
        XCTAssertTrue(commands[1].exists, "按前缀过滤把匹配的命令也滤掉了")
        field.typeText(XCUIKeyboardKey.delete.rawValue)
        tap(commands[0])
        XCTAssertTrue(text(startingWith: "/help").waitForExistence(timeout: 5), "点了命令，时间线末尾没有这条消息")
        XCTAssertTrue(commands[0].waitForNonExistence(timeout: 5), "发出去以后列表没收起")
        XCTAssertEqual(field.value as? String, "/", "发出去以后输入框里的草稿没了")

        tap(app.buttons["composerCommands"])
        XCTAssertTrue(commands[2].waitForExistence(timeout: 5), "“菜单”按钮没打开命令列表")
    }

    /// 对方正在输入时顶栏状态行换成“正在输入…”。
    func testTopBarShowsTyping() {
        enter(Self.directRoom)
        XCTAssertTrue(app.staticTexts["barActivity"].waitForExistence(timeout: 5), "顶栏没有正在输入\n\(app.debugDescription)")
        waitForFloatingDateToFade()
        shot("direct")
    }

    /// 输入框空着时没有发送键，打了字才出现；“菜单”胶囊一直在。
    func testSendButtonAppearsWithText() {
        enter(Self.directRoom)
        let field = input
        XCTAssertFalse(app.buttons["composerSend"].exists, "输入框空着就有发送键")
        XCTAssertTrue(app.buttons["composerCommands"].exists, "Bot 有命令，输入栏没有“菜单”")
        field.tap()
        waitForFloatingDateToFade()
        shot("composer-empty")
        focus(field)
        field.typeText("Hello")
        XCTAssertTrue(app.buttons["composerSend"].waitForExistence(timeout: 5), "打了字没有发送键")
        XCTAssertTrue(app.buttons["composerCommands"].exists, "打了字“菜单”没了")
        shot("composer-text")
    }

    /// 长按一条消息出系统菜单（ADR-0029），点“回复”进入回复状态。
    func testLongPressShowsMenu() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        message(3).press(forDuration: 1.2)
        let reply = app.buttons["messageReply"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5), "长按没出菜单\n\(app.debugDescription)")
        // 菜单弹出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
        shot("context-menu")
        reply.tap()
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "点了“回复”没进入回复状态")
    }

    /// 深色模式下的私聊、旁聊和群聊：只留截图给人看，断言时间线照常出来。
    func testDarkAppearance() {
        app.terminate()
        XCUIDevice.shared.appearance = .dark
        addTeardownBlock { XCUIDevice.shared.appearance = .light }
        UITestBackend.launch(app)
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        shot("direct-dark")
        let (first, second) = openAttachSheet()
        shot("attach-tabs-dark")
        _ = selectTwo(first, second)
        shot("attach-selected-dark")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.directRoom)
        openSideChat()
        shot("side-chat-dark")
        let photo = openSideAttach()
        shot("side-chat-attach-dark")
        for index in 0..<3 {
            let cell = app.buttons["attachPhoto.\(index)"]
            XCTAssertTrue(cell.waitForExistence(timeout: 10), "相册网格没铺出第 \(index) 张")
            cell.tap()
        }
        XCTAssertTrue(wait(5) { photo.isSelected }, "深色下旁聊的相册选不上照片")
        shot("side-chat-multi-dark")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.directRoom)
        openForwardSheet()
        selectTwoRooms()
        shot("forward-sheet-dark")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.groupRoom)
        waitForFloatingDateToFade()
        shot("group-dark")
    }

    /// 顶栏开新旁聊：写第一句发出去，面板收完盖层才上来，第一句落在这间旁聊里。
    func testNewSideChatFromTopBar() {
        enter(Self.directRoom)
        tap(app.buttons["sideChatNew"])
        let field = app.textFields["sideChatFirstMessage"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "顶栏没打开新旁聊的面板\n\(app.debugDescription)")
        field.tap()
        field.typeText("Hello side")
        tap(app.buttons["sideChatStart"])
        XCTAssertTrue(app.buttons["barBack"].waitForExistence(timeout: 10), "发完第一句没进旁聊\n\(app.debugDescription)")
        XCTAssertTrue(text(startingWith: "Hello side").waitForExistence(timeout: 10), "第一句没发进旁聊")
        XCTAssertFalse(app.textViews["Message"].firstMatch.isHittable, "旁聊没盖住主对话")
        shot("side-chat-new")
    }

    /// 点主对话里的旁聊卡片进去：上面盖的是这间旁聊的对话，不是退回聊天列表；返回键收掉旁聊、主对话回来。
    func testSideChatOpens() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let last = text(startingWith: "Fixture side 3")
        XCTAssertTrue(last.exists, "旁聊里没有最后一条回复")
        XCTAssertTrue(app.buttons["barBack"].exists, "旁聊顶栏没有返回")
        shot("side-chat")
        tap(app.buttons["barBack"])
        // 盖层收到一半时的画面：内容要被 `onDismiss` 留到转场跑完，不能这一下就抽走。
        shot("side-chat-back")
        XCTAssertTrue(last.waitForNonExistence(timeout: 10), "返回以后旁聊的内容还在")
        XCTAssertFalse(app.buttons["barBack"].exists, "返回以后旁聊的顶栏还在")
        XCTAssertTrue(wait(10) { app.textViews["Message"].firstMatch.isHittable }, "返回以后主对话的输入栏没回来")
        XCTAssertTrue(app.buttons["sideChatCard.$ui-fake-5"].exists, "返回以后主对话没回来")
    }

    /// 旁聊的附件面板只有相册：没有“文件”那一项，照片照常出得来。
    func testSideChatAttachShowsPhotosOnly() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        _ = openSideAttach()
        XCTAssertFalse(app.buttons["attachFiles"].exists, "旁聊的附件面板里还有“文件”")
        shot("side-chat-attach")
    }

    /// 旁聊的相册点得动：照片不是灰的，选一张、点“完成”，照片真的发进旁聊。
    func testSideChatPhotoGridUsable() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let first = openSideAttach()
        XCTAssertTrue(first.isEnabled, "旁聊里照片点不了，上限还是 0 个")
        first.tap()
        XCTAssertTrue(first.isSelected, "点了的照片没变成选中")
        sendPickedPhotos()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了“完成”面板没收起")
        XCTAssertTrue(waitForCount(photoEchoes, 1, timeout: 10), "选的照片没发进旁聊")
        XCTAssertFalse(app.buttons["dialogOk"].exists, "发照片弹了错误")
    }

    /// 旁聊一次能选好几张：三张都在一张面板里选上，不用开三次。
    func testSideChatPicksSeveralPhotosAtOnce() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        XCTAssertTrue(openSideAttach().isEnabled, "旁聊里照片点不了，上限还是 0 个")
        let photos = (0..<3).map { app.buttons["attachPhoto.\($0)"] }
        XCTAssertTrue(photos[2].waitForExistence(timeout: 15), "旁聊的附件面板里没有三张照片\n\(app.debugDescription)")
        for photo in photos { photo.tap() }
        XCTAssertTrue(wait(5) { photos.allSatisfy { $0.isSelected } }, "一张面板里选不上三张")
        XCTAssertTrue(app.buttons["attachSend"].exists, "选了三张没有“完成”")
        shot("side-chat-multi")
        sendPickedPhotos()
        XCTAssertTrue(waitForCount(photoEchoes, 3, timeout: 20), "三张照片没有都发进旁聊")
        XCTAssertFalse(app.buttons["dialogOk"].exists, "发照片弹了错误")
    }

    /// 旁聊的说明只跟第一张照片：发两张时第一张带说明、第二张不带，草稿清掉。
    func testSideChatCaptionGoesOnlyWithFirstPhoto() {
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let field = input
        focus(field)
        field.typeText("Fixture first caption")
        XCTAssertTrue(openSideAttach().isEnabled, "旁聊里照片点不了，上限还是 0 个")
        let photos = (0..<2).map { app.buttons["attachPhoto.\($0)"] }
        XCTAssertTrue(photos[1].waitForExistence(timeout: 15), "旁聊的附件面板里没有两张照片\n\(app.debugDescription)")
        for photo in photos { photo.tap() }
        XCTAssertTrue(wait(5) { photos.allSatisfy { $0.isSelected } }, "一张面板里选不上两张")
        sendPickedPhotos()
        XCTAssertTrue(
            text(startingWith: "Fixture first caption").waitForExistence(timeout: 20),
            "第一张没带上说明\n\(app.debugDescription)"
        )
        XCTAssertTrue(text(startingWith: "photo").waitForExistence(timeout: 20), "第二张没发出去\n\(app.debugDescription)")
        let captions = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture first caption"))
        XCTAssertEqual(captions.count, 1, "说明跟了不止一张\n\(app.debugDescription)")
        XCTAssertEqual(field.value as? String, "", "说明随照片发出去了，草稿没清")
    }

    /// 第一张被拒：说明留在草稿里，第二张照发但不带说明。
    func testSideChatRefusedFirstPhotoKeepsCaptionForLater() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_FAIL_SEND"] = "1"
        app.launch()
        enter(Self.directRoom)
        waitForFloatingDateToFade()
        openSideChat()
        let field = input
        focus(field)
        field.typeText("Fixture kept caption")
        XCTAssertTrue(openSideAttach().isEnabled, "旁聊里照片点不了，上限还是 0 个")
        let photos = (0..<2).map { app.buttons["attachPhoto.\($0)"] }
        XCTAssertTrue(photos[1].waitForExistence(timeout: 15), "旁聊的附件面板里没有两张照片\n\(app.debugDescription)")
        for photo in photos { photo.tap() }
        XCTAssertTrue(wait(5) { photos.allSatisfy { $0.isSelected } }, "一张面板里选不上两张")
        sendPickedPhotos()
        XCTAssertTrue(text(startingWith: "photo").waitForExistence(timeout: 20), "第一张被拒以后第二张没发出去\n\(app.debugDescription)")
        XCTAssertFalse(text(startingWith: "Fixture kept caption").exists, "说明跟了第二张，本该留在草稿里\n\(app.debugDescription)")
        XCTAssertEqual(field.value as? String, "Fixture kept caption", "第一张被拒后草稿没了")
    }

    /// 宽表格上横拖不抢回复：从右往左拖是表格自己滚，回复条不出现；从左往右拖才进回复。
    func testTableScrollsSidewaysWithoutStartingReply() {
        enter(Self.directRoom)
        let cell = text(startingWith: "Fixture cell 0-0")
        tap(app.buttons["timelineDown"])
        reveal(cell)
        waitForFloatingDateToFade()
        shot("table-before")
        for _ in 0..<3 {
            let start = cell.coordinate(withNormalizedOffset: CGVector(dx: 0.4, dy: 0.5))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 160, dy: 0)),
                        withVelocity: .default, thenHoldForDuration: 0.1)
            XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "表格上往右拖没有进回复\n\(app.debugDescription)")
            app.buttons["replyCancel"].tap()
        }
        shot("table-replied")
        for _ in 0..<3 {
            let before = cell.frame.minX
            let start = cell.coordinate(withNormalizedOffset: CGVector(dx: 0.6, dy: 0.5))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: -120, dy: 0)),
                        withVelocity: .default, thenHoldForDuration: 0.1)
            XCTAssertFalse(app.buttons["replyCancel"].exists, "表格上往左拖进了回复\n\(app.debugDescription)")
            XCTAssertLessThan(cell.frame.minX, before - 20, "表格没跟着往左滚")
        }
        shot("table-scrolled")
    }

    /// 最大字号下，房间名的胶囊和带两个选项的按钮还在框里：文字不截断、按钮不裁字。
    func testLargestTextKeepsBarAndChoicesWhole() {
        app.terminate()
        app.launchArguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        // 深色轮次由跑测脚本经 `TEST_RUNNER_WONTON_DARK=1` 传进来；XCUITest 每轮开场会把设备外观改回上一次的样子，
        // 只在外面 `simctl ui` 切会被盖回去。
        if ProcessInfo.processInfo.environment["WONTON_DARK"] == "1" {
            XCUIDevice.shared.appearance = .dark
        }
        UITestBackend.launch(app)
        enter(Self.groupRoom)
        let title = app.staticTexts["Family"]
        XCTAssertTrue(title.waitForExistence(timeout: 10), "顶栏没有房间名\n\(app.debugDescription)")
        shot("large-text-bar")
        XCTAssertEqual(title.label, "Family", "房间名被改写了")
        XCTAssertLessThan(title.frame.maxX, app.windows.element(boundBy: 0).frame.maxX, "房间名顶出了屏幕")
        tap(app.buttons["timelineDown"])
        let approve = app.buttons["messageChoice.✅"]
        reveal(approve)
        waitForFloatingDateToFade()
        shot("large-text-choices")
        for (button, label) in [(approve, "approve once"), (app.buttons["messageChoice.❌"], "deny")] {
            XCTAssertTrue(button.exists, "\(label) 的选项按钮不见了")
            let inside = app.staticTexts[label].firstMatch
            XCTAssertTrue(inside.exists, "\(label) 的文字不见了")
            XCTAssertLessThanOrEqual(inside.frame.maxY, button.frame.maxY + 0.5, "\(label) 的字被按钮裁掉")
            XCTAssertLessThanOrEqual(inside.frame.maxX, button.frame.maxX + 0.5, "\(label) 的字横着出框")
        }
    }

    /// 房间顶栏没有“打开面板”的按钮；点头像进 Bot 资料页，从那里打开这个房间的面板（ADR-0052）。
    func testPanelOpensFromBotProfile() {
        enter(Self.directRoom)
        XCTAssertFalse(app.buttons["panelOpen"].exists, "房间顶栏还有打开面板的按钮")
        tap(app.buttons["roomTopBarAvatar"])
        let open = app.buttons["botProfilePanelOpen"]
        for _ in 0..<4 where !open.isHittable {
            app.swipeUp()
        }
        tap(open)
        XCTAssertTrue(app.otherElements["chatPanel"].waitForExistence(timeout: 5), "资料页的按钮没有打开面板\n\(app.debugDescription)")
    }

    /// 回形针打开附件面板：相册的照片网格能点选几张，底下浮着一排切换，私聊里有相册和文件；选了照片那一排换成附言框和
    /// 发送键，关掉回到那一排；写了附言点发送，面板收起。
    func testAttachSheetTabsThenCaptionAndSend() {
        enter(Self.directRoom)
        let (first, second) = openAttachSheet()
        XCTAssertTrue(app.buttons["attachPhotos"].exists, "附件面板底下没有“相册”")
        XCTAssertTrue(app.buttons["attachFiles"].exists, "私聊的附件面板底下没有“文件”")
        XCTAssertFalse(app.buttons["attachSend"].exists, "还没选就有发送键")
        shot("attach-sheet")
        let send = selectTwo(first, second)
        XCTAssertFalse(app.buttons["attachFiles"].exists, "选了照片，那一排切换还在")
        shot("attach-selected")
        tap(app.buttons["attachCancel"])
        XCTAssertTrue(app.buttons["attachFiles"].waitForExistence(timeout: 5), "取消选择没回到那一排切换")
        XCTAssertFalse(first.isSelected || second.isSelected, "取消选择后照片还是选中的")
        first.tap()
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture caption")
        XCTAssertTrue(wait(5) { send.isHittable }, "键盘弹起后发送键点不到")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
    }

    /// 转发：面板顶上能搜，会话能选几个；选了以后附言框和发送键贴着面板底、点得到，写了附言点发送面板收起。
    func testForwardSheetSelectsSeveralAndSendsWithComment() {
        enter(Self.directRoom)
        openForwardSheet()
        let send = app.buttons["forwardSend"]
        XCTAssertFalse(send.exists, "还没选就有发送键")
        selectTwoRooms()
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了会话，发送键不在屏幕内\n\(app.debugDescription)")
        let comment = element("forwardCaption")
        XCTAssertTrue(wait(5) { comment.exists && comment.isHittable }, "选了会话，附言框不在屏幕内\n\(app.debugDescription)")
        focus(comment)
        comment.typeText("Fixture comment")
        XCTAssertTrue(wait(5) { send.isHittable }, "键盘弹起后发送键点不到")
        shot("forward-sheet")
        send.tap()
        XCTAssertTrue(app.textFields["forwardSearch"].waitForNonExistence(timeout: 5), "发送以后面板没收起")
    }

    /// 附件面板里发出去的照片和附言真到了时间线：附言成了一条消息，附件区空了（ADR-0109 第 9 节）。
    func testAttachSendsPhotoAndCaption() {
        enter(Self.directRoom)
        let (first, _) = openAttachSheet()
        first.tap()
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键")
        XCTAssertTrue(send.isEnabled, "选了照片发送键还是灰的")
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture attach caption")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
        XCTAssertTrue(text(startingWith: "Fixture attach caption").waitForExistence(timeout: 10),
                      "附言没进时间线\n\(app.debugDescription)")
        XCTAssertTrue(element("composerTray").waitForNonExistence(timeout: 10), "发出去了附件区里还留着照片")
        shot("attach-sent")
        darkShot("attach-sent-dark")
    }

    /// 发送被拒：面板照常收起，附言、回复对象和附件区里的照片都留着，让主人重试（ADR-0109 第 9 节）。
    func testRejectedSendKeepsCaptionPhotoAndReply() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_TEST_FAIL_ATTACHMENT"] = "1"
        UITestBackend.launch(app)
        enter(Self.directRoom)
        // 旁聊里是另一份假时间线（Fixture side N），回复落点旁边那条。
        startReply(on: text(startingWith: "Fixture side 1"))
        let (first, _) = openAttachSheet()
        first.tap()
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键")
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture kept caption")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
        XCTAssertTrue(element("composerTray").waitForExistence(timeout: 5), "被拒以后附件区里的照片没了")
        XCTAssertEqual(input.value as? String, "Fixture kept caption", "被拒以后附言被清掉了")
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒以后回复对象没了")
        shot("attach-rejected")
        darkShot("attach-rejected-dark")
    }

    /// 转发时写了附言：附言和转发的原消息都进了对方房间，附言排在上面（ADR-0073 第 4 节、ADR-0109 第 9 节）。
    func testForwardCommentLandsInTargetRoom() {
        enter(Self.directRoom)
        openForwardSheet()
        let search = app.textFields["forwardSearch"]
        focus(search)
        search.typeText("Notes")
        tap(app.buttons["forward-!ui-fake-notes:home.example"])
        let comment = element("forwardCaption")
        XCTAssertTrue(wait(5) { comment.exists && comment.isHittable }, "选了会话，附言框不在屏幕内\n\(app.debugDescription)")
        focus(comment)
        comment.typeText("Fixture forward comment")
        XCTAssertEqual(comment.value as? String, "Fixture forward comment", "附言没打进附言框")
        tap(app.buttons["forwardSend"])
        // 附言在目标房间没打开时排着，打开那条才发出去。
        XCTAssertTrue(app.buttons["forwardSend"].waitForNonExistence(timeout: 5), "转发面板没收起")
        tap(app.buttons["Back"])
        XCTAssertTrue(app.buttons["chatRoom.!ui-fake-notes:home.example"].waitForExistence(timeout: 10),
                      "没回到会话列表\n\(app.debugDescription)")
        enter("chatRoom.!ui-fake-notes:home.example")
        jumpToNewest()
        // 副本是最后一条、比整屏还高，贴底时它就在屏上；附言在它上面一条，往回翻一点就进树。
        // 原件滚到顶上去了，所以不能按“两条 Fixture message 4”数，按先后比。
        let copy = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 4")).firstMatch
        let arrived = text(startingWith: "Fixture forward comment")
        revealBackwards(arrived)
        XCTAssertTrue(arrived.exists, "附言没进目标房间\n\(app.debugDescription)")
        XCTAssertTrue(copy.exists, "附言下面没有转发的消息\n\(app.debugDescription)")
        XCTAssertLessThan(arrived.frame.minY, copy.frame.minY, "转发的消息没排在附言下面")
        shot("forwarded-comment")
        darkShot("forwarded-comment-dark")
    }

    /// 转发带附言发给两个会话：两边各有一条附言压在自己的副本上面（ADR-0073 第 4 节、ADR-0109 第 9 节）。
    /// 顺序按时间线里两条消息的先后比，只看都在不够。
    func testForwardCommentToEveryRoomLandsAboveItsCopy() {
        enter(Self.directRoom)
        openForwardSheet()
        let search = app.textFields["forwardSearch"]
        focus(search)
        search.typeText("am")
        tap(app.buttons["forward-!ui-fake-call:home.example"])
        // 清掉搜索再找另一个房间。
        focus(search)
        search.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 2))
        search.typeText("Notes")
        tap(app.buttons["forward-!ui-fake-notes:home.example"])
        let comment = element("forwardCaption")
        XCTAssertTrue(wait(5) { comment.exists && comment.isHittable }, "选了会话，附言框不在屏幕内\n\(app.debugDescription)")
        focus(comment)
        comment.typeText("Fixture forward comment")
        tap(app.buttons["forwardSend"])
        XCTAssertTrue(app.textFields["forwardSearch"].waitForNonExistence(timeout: 5), "转发面板没收起")
        for (id, name) in [("!ui-fake-call:home.example", "Family"), ("!ui-fake-notes:home.example", "Notes")] {
            tap(app.buttons["Back"])
            XCTAssertTrue(app.buttons["chatRoom.\(id)"].waitForExistence(timeout: 10),
                          "没回到会话列表\n\(app.debugDescription)")
            enter("chatRoom.\(id)")
            jumpToNewest()
            // 副本是最后一条、比整屏还高，贴底时它就在屏上；附言在它上面一条，往回翻一点就进树。
            let copy = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message 4")).firstMatch
            let arrived = text(startingWith: "Fixture forward comment")
            revealBackwards(arrived)
            XCTAssertTrue(arrived.exists, "\(name) 里没有附言\n\(app.debugDescription)")
            XCTAssertTrue(copy.exists, "\(name) 里没有副本\n\(app.debugDescription)")
            XCTAssertLessThan(arrived.frame.minY, copy.frame.minY, "\(name) 里附言没排在副本上面")
            shot("forward-comment-\(name.lowercased())")
        }
    }

    /// 照片的字节还在取的时候就显示“正在发送”：面板已经收起，等待期间打的字发完留在输入框里（ADR-0109 第 9 节）。
    func testPhotoBytesWaitShowsSendingAndKeepsTextTypedMeanwhile() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_PHOTO_DELAY_MS"] = "4000"
        UITestBackend.launch(app)
        enter(Self.directRoom)
        let (first, _) = openAttachSheet()
        first.tap()
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键")
        let caption = element("attachCaption")
        focus(caption)
        caption.typeText("Fixture caption before")
        send.tap()
        XCTAssertTrue(first.waitForNonExistence(timeout: 5), "点了发送面板没收起")
        XCTAssertTrue(element("composerSending").waitForExistence(timeout: 5), "取照片的时候没有“正在发送”")
        let field = input
        focus(field)
        field.typeText("typed while sending")
        XCTAssertTrue(element("composerSending").waitForNonExistence(timeout: 20), "照片发完了“正在发送”还在")
        XCTAssertTrue(((field.value as? String) ?? "").contains("typed while sending"), "等待期间打的字没了")
    }

    /// 旁聊里被拒：草稿和回复对象都留着；提示上的关闭能把它收掉（ADR-0093、ADR-0109 第 9 节）。
    func testSideChatRejectedSendKeepsDraftAndReply() {
        app.terminate()
        app.launchEnvironment["WONTON_UI_FAIL_SEND"] = "1"
        UITestBackend.launch(app)
        enter(Self.directRoom)
        // 卡片就挂在进房落点那条：没画出来往上挪一两段，不用横穿整条时间线；
        // 底下可能被附件提示盖住，交给 revealClearOfHint 挪到提示上面。
        let card = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "sideChatCard.")).firstMatch
        for _ in 0..<3 where !card.exists { dragDown() }
        XCTAssertTrue(revealClearOfHint(card), "翻不到露出来的旁聊卡片\n\(app.debugDescription)")
        tap(card)
        // 假数据里旁聊和主对话共用同一份时间线，卡片在盖层里也还在；靠提示认出已经进了旁聊。
        XCTAssertTrue(element("sideChatAttachHint").waitForExistence(timeout: 10), "点了卡片没进旁聊\n\(app.debugDescription)")
        shot("side-chat-hint")
        darkShot("side-chat-hint-dark")
        // 旁聊里是它自己的假时间线（`Fixture side N`），回复紧挨着落点的那条。
        startReply(on: text(startingWith: "Fixture side 1"))
        let field = input
        focus(field)
        field.typeText("Fixture side draft")
        tap(app.buttons["composerSend"])
        XCTAssertTrue(app.buttons["replyCancel"].exists, "被拒以后回复对象没了")
        XCTAssertEqual(field.value as? String, "Fixture side draft", "被拒以后草稿被清掉了")
        shot("side-chat-rejected")
        tap(app.buttons["sideChatHintClose"])
        XCTAssertTrue(element("sideChatAttachHint").waitForNonExistence(timeout: 5), "提示上的关闭没把它收掉")
    }

    /// 引用块只有名字加一两行摘要那么高，不跟着气泡撑开。
    func testQuoteBlockHugsItsContent() {
        enter(Self.directRoom)
        let quote = app.buttons["messageQuote.ui-fake-20"]
        tap(app.buttons["timelineDown"])
        reveal(quote)
        waitForFloatingDateToFade()
        shot("long-reply")
        XCTAssertLessThanOrEqual(quote.frame.height, Self.quoteMaxHeight, "引用块太高：\(quote.frame)")
    }

    /// 长消息每一段都完整显示，不按行数截断；时间只落在最后一段的末行，不压着别的字。
    func testLongReplyShowsEveryLineAndTimeStaysClear() {
        enter(Self.directRoom)
        tap(app.buttons["timelineDown"])
        reveal(app.buttons["messageQuote.ui-fake-20"])
        let paragraphs = Self.longReplyParagraphs.map { text(startingWith: $0.prefix) }
        for (paragraph, expected) in zip(paragraphs, Self.longReplyParagraphs) {
            XCTAssertTrue(paragraph.waitForExistence(timeout: 5), "找不到“\(expected.prefix)”这一段")
            // 一个汉字宽不超过正文字号：这一段至少要这么多行。被截断的段行数不够。
            let lines = (CGFloat(expected.hanCount) * Self.hanWidth / paragraph.frame.width).rounded(.up)
            XCTAssertGreaterThanOrEqual(
                paragraph.frame.height, lines * Self.lineHeight - 2,
                "“\(expected.prefix)”这一段被截断：\(paragraph.frame)，至少要 \(lines) 行"
            )
        }
        let last = paragraphs[paragraphs.count - 1].frame
        let metas = app.descendants(matching: .any).matching(identifier: "messageMeta").allElementsBoundByIndex
            .map { $0.frame }.filter { $0.intersects(last) }
        XCTAssertEqual(metas.count, 1, "最后一段上应该正好有一处时间：\(metas)")
        for meta in metas {
            XCTAssertGreaterThanOrEqual(meta.minY, last.maxY - Self.lineHeight * 1.5, "时间不在最后一段的末行：\(meta) \(last)")
            for paragraph in paragraphs.dropLast() {
                XCTAssertFalse(meta.intersects(paragraph.frame), "时间压在前面的段上：\(meta) \(paragraph.frame)")
            }
        }
    }

    /// 假房间第 20 条（核心的 `ui_fake_text/long_reply.txt`）每段开头几个字和汉字个数的下限。
    private static let longReplyParagraphs: [(prefix: String, hanCount: Int)] = [
        ("没有出问题", 20), ("拖得久的原因", 80), ("有一件我确实做不到的事", 50), ("你在 App 里打开", 40)
    ]
    /// 默认字号下正文的字宽下限和行高（`typography.message` 是 16 / 21，字宽留一点余量）。
    private static let hanWidth: CGFloat = 15
    private static let lineHeight: CGFloat = 21
    /// 引用块最高：名字一行（18）加摘要两行（各 20），再留一点余量。
    private static let quoteMaxHeight: CGFloat = 64

    /// 群聊里别人的组尾旁边有头像，私聊里没有。
    func testGroupShowsSenderAvatarAndDirectDoesNot() {
        enter(Self.groupRoom)
        let avatar = app.descendants(matching: .any).matching(identifier: "messageAvatar").firstMatch
        XCTAssertTrue(avatar.waitForExistence(timeout: 5), "群聊里没有发送者头像\n\(app.debugDescription)")
        // 翻到最上面：顶栏下的 AI 那一行占着自己的位置，第一条气泡排在它下面，不被它盖住。
        let aiLine = app.descendants(matching: .any)["roomAiLine"]
        XCTAssertTrue(aiLine.waitForExistence(timeout: 5), "群聊顶栏下没有 AI 那一行\n\(app.debugDescription)")
        let first = message(0)
        reveal(first)
        for _ in 0..<3 { dragDown() }
        XCTAssertTrue(wait(5) { first.exists && first.frame.minY >= aiLine.frame.maxY },
                      "第一条气泡被 AI 那一行盖住\n\(app.debugDescription)")
        waitForFloatingDateToFade()
        shot("group-top")
        // 再进一次，停在最新一条：群聊平常的样子。
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.groupRoom)
        waitForFloatingDateToFade()
        shot("group")
        app.terminate()
        UITestBackend.launch(app)
        enter(Self.directRoom)
        XCTAssertFalse(avatar.exists, "私聊里画了发送者头像")
    }

    // MARK: 助手

    /// 点回形针打开附件面板，等相册网格出来，给回前两张照片。
    private func openAttachSheet(file: StaticString = #filePath, line: UInt = #line) -> (XCUIElement, XCUIElement) {
        tap(app.buttons["composerAttach"], file: file, line: line)
        // 第一次打开网格系统会问相册权限。
        // 系统弹窗跟着模拟器的语言走，不认字：三个按钮里中间那个是“允许完全访问”。
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        let first = app.buttons["attachPhoto.0"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        XCTAssertTrue(first.waitForExistence(timeout: 15), "附件面板里没有照片\n\(app.debugDescription)", file: file, line: line)
        // 面板升上来的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
        return (first, app.buttons["attachPhoto.1"])
    }

    /// 选两张照片，等附言框和发送键出来，给回发送键。
    private func selectTwo(_ first: XCUIElement, _ second: XCUIElement) -> XCUIElement {
        first.tap()
        second.tap()
        XCTAssertTrue(wait(5) { first.isSelected && second.isSelected }, "点了的照片没变成选中")
        let send = app.buttons["attachSend"]
        XCTAssertTrue(wait(5) { send.exists && send.isHittable }, "选了照片没有发送键\n\(app.debugDescription)")
        XCTAssertTrue(element("attachCaption").exists, "选了照片没有附言框")
        Thread.sleep(forTimeInterval: 0.5)
        return send
    }

    /// 同一处再拍一张深色的，拍完换回浅色。
    private func darkShot(_ name: String) {
        XCUIDevice.shared.appearance = .dark
        waitForFloatingDateToFade()
        shot(name)
        XCUIDevice.shared.appearance = .light
    }

    /// 长按第几条消息点“回复”，等回复条出来。
    private func startReply(at index: Int, file: StaticString = #filePath, line: UInt = #line) {
        startReply(on: message(index), file: file, line: line)
    }

    /// 长按指到的消息点“回复”，等回复条出来。
    private func startReply(on element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        reveal(element, file: file, line: line)
        waitForFloatingDateToFade()
        element.press(forDuration: 1.2)
        let reply = app.buttons["messageReply"]
        guard reply.waitForExistence(timeout: 5) else {
            return XCTFail("长按菜单里没有“回复”\n\(app.debugDescription)", file: file, line: line)
        }
        reply.tap()
        XCTAssertTrue(app.buttons["replyCancel"].waitForExistence(timeout: 5), "点了“回复”没进入回复状态", file: file, line: line)
    }

    /// 长按一条消息点“转发”，等转发面板出来。转第 4 条：第 3 条是照片，假数据只转发文字（`text_of` 拿不到照片的正文）。
    private func openForwardSheet() {
        waitForFloatingDateToFade()
        message(4).press(forDuration: 1.2)
        let forward = app.buttons["messageForward"]
        XCTAssertTrue(forward.waitForExistence(timeout: 5), "长按菜单里没有“转发”")
        forward.tap()
        XCTAssertTrue(app.textFields["forwardSearch"].waitForExistence(timeout: 5), "转发面板没有搜索框")
    }

    /// 在转发面板里选前两个会话。
    private func selectTwoRooms() {
        let rooms = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "forward-"))
        XCTAssertGreaterThanOrEqual(rooms.count, 2, "转发面板里的会话不到两个")
        rooms.element(boundBy: 0).tap()
        rooms.element(boundBy: 1).tap()
        XCTAssertTrue(app.buttons["forwardSend"].waitForExistence(timeout: 5), "选了会话没有发送键")
        Thread.sleep(forTimeInterval: 0.5)
    }

    /// 按无障碍标识找，不管它是哪种控件（多行的输入框在无障碍树里是 textView）。
    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    /// 浮动日期只在滚动时盖在消息上，停下后淡出，不压着字（ADR-0109 第 4 节）。
    private func waitForFloatingDateToFade(file: StaticString = #filePath, line: UInt = #line) {
        let floatingDate = app.descendants(matching: .any)["timelineFloatingDate"]
        XCTAssertTrue(floatingDate.waitForNonExistence(timeout: 5), "停止滚动后浮动日期没淡出", file: file, line: line)
        // 无障碍树里先没了，淡出的动画还要一会儿；截图等它画完。
        Thread.sleep(forTimeInterval: 1)
    }

    private func enter(_ room: String) {
        tap(app.buttons[room])
        // 房间停在顶上还是贴底由落点和贴底决定：等到任意一条假消息就是时间线到了。
        let any = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture ")).firstMatch
        XCTAssertTrue(any.waitForExistence(timeout: 20), "时间线没出来\n\(app.debugDescription)")
        XCTAssertTrue(input.exists, "房间打开了，输入框没出来")
    }

    /// 假房间的第几条消息（正文是 “Fixture message N”）。
    private func message(_ index: Int) -> XCUIElement {
        text(startingWith: "Fixture message \(index)")
    }

    /// 第几条消息自己在屏上、点得到。引用里的摘要和原消息是一样的字，落在引用里的不算。
    private func originalOnScreen(_ index: Int, besides quote: XCUIElement) -> Bool {
        let inQuote = quote.exists ? quote.frame : .null
        return app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture message \(index)"))
            .allElementsBoundByIndex
            .contains { inView($0) && !inQuote.contains(CGPoint(x: $0.frame.midX, y: $0.frame.midY)) }
    }

    private func text(startingWith prefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    /// 输入框。盖层开着时主对话那份也在无障碍树里，取第一份。
    private var input: XCUIElement { app.textViews["Message"].firstMatch }

    /// 列表可见区的上下沿：顶栏那一刻（有人打字时量提示的下沿）到输入框那一刻（上面有提示、附件区时量它们的上沿）。
    private func visibleBounds() -> (top: CGFloat, bottom: CGFloat) {
        let activity = app.staticTexts["barActivity"].firstMatch
        let top = activity.exists ? activity.frame.maxY : app.buttons["roomTopBarAvatar"].firstMatch.frame.maxY
        let cover = [element("sideChatAttachHint"), element("composerTray")].filter { $0.exists }.map { $0.frame.minY }
        let bottom = min(input.frame.minY, cover.min() ?? .infinity)
        return (top, bottom)
    }

    /// 整个落在顶栏和输入框之间。列表垫在半透明的顶栏下面，那里的元素 `isHittable` 也是真，点下去点到的是顶栏；
    /// 输入框上面的附件区和旁聊提示一样盖住列表。
    private func inView(_ node: XCUIElement) -> Bool {
        guard node.exists, node.isHittable else { return false }
        let (top, bottom) = visibleBounds()
        return node.frame.minY > top && node.frame.maxY < bottom
    }

    /// 翻到露在附件提示上面的位置：卡片在提示底下就往上挪，在提示上面更远处就往下找。
    private func revealClearOfHint(_ node: XCUIElement) -> Bool {
        for _ in 0..<60 {
            guard node.exists else {
                dragDown()
                continue
            }
            let (top, bottom) = visibleBounds()
            if node.frame.minY > top, node.frame.maxY < bottom, node.isHittable { return true }
            if node.frame.maxY <= top {
                dragDown()
            } else {
                dragUp()
            }
        }
        return false
    }

    /// `ScrollDebug` 里的闪光计数；读不到给 -1。
    private func flashCount(_ element: XCUIElement) -> Int {
        let part = element.label.split(separator: " ").first { $0.hasPrefix("flashes=") }
        return part.flatMap { Int($0.dropFirst("flashes=".count)) } ?? -1
    }

    /// 一段一段翻到元素整个在可见区里。默认往回翻；元素已经画在输入框下面时朝新消息翻，
    /// 房间开在落点上，紧贴着落点下面的东西（旁聊卡）不用先翻到最上面再回来。
    private func reveal(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for step in 0..<200 {
            if inView(element) { return }
            if element.exists {
                // 已经画出来、但在输入框下面（朝新消息的一侧）就往下翻，其余往回翻。
                // 旁聊页开着时树里有两个 “Message”，走 `input` 取第一份。
                if element.frame.minY > input.frame.minY { dragUp() } else { dragDown() }
            } else if step < 12 {
                // 没画出来就按锚点猜：房间开在落点上，落点下面紧贴的卡（旁聊卡）要先往回翻。
                dragDown()
            } else {
                // 猜的这头翻到头了还没有：换一头。翻到头时列表回弹，两边都试满才判失败。
                if step % 10 < 5 { dragDown() } else { dragUp() }
            }
        }
        XCTFail("翻了 200 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 回到底部：有“向下”就一直点，点没了就是贴底了。
    private func jumpToNewest() {
        for _ in 0..<20 where app.buttons["timelineDown"].exists { app.buttons["timelineDown"].tap() }
    }

    /// 从底部往回翻到 `element` 进了树为止。长消息比整屏还高、命不中，只看存在性。
    private func revealBackwards(_ element: XCUIElement) {
        for _ in 0..<40 where !element.exists { dragDown() }
    }

    /// 点一下输入框，等它真拿到键盘焦点；没拿到就重试。焦点还在上一个框里时 typeText 会打进那个框。
    private func focus(_ field: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for _ in 0..<5 {
            field.tap()
            // XCUITest 没有公开的焦点 API，`hasKeyboardFocus` 走 KVC 读。
            if (field.value(forKey: "hasKeyboardFocus") as? Bool) == true { return }
        }
        XCTFail("输入框一直没拿到键盘焦点\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往新的一头挪一小段：从提示上面起手，挪 110 点，不会把整屏翻过头。
    private func dragUp() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.55))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.42)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 大片地翻到能点元素：房间开在进来的那条附近，旁聊卡片就在两三条之后，先往新消息那边翻，到底再反过来。
    /// 一次拖大半屏，按能不能点判断，比逐条消息找快。
    private func scrollTo(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        for backwards in [false, true] {
            for _ in 0..<30 {
                if element.isHittable { return }
                let from = backwards ? CGVector(dx: 0.5, dy: 0.15) : CGVector(dx: 0.5, dy: 0.9)
                let to = backwards ? CGVector(dx: 0.5, dy: 0.9) : CGVector(dx: 0.5, dy: 0.15)
                app.coordinate(withNormalizedOffset: from)
                    .press(forDuration: 0.01, thenDragTo: app.coordinate(withNormalizedOffset: to),
                           withVelocity: .default, thenHoldForDuration: 0)
            }
        }
        XCTFail("两头各翻 30 段也没找到 \(element)\n\(app.debugDescription)", file: file, line: line)
    }

    /// 往回翻一段。慢拖、松手前停一下：不带惯性，不会一下翻过头。
    private func dragDown() {
        // 起点放在下半屏：顶栏下叠着插件横幅时，0.35 会压在那张卡片上，拖不动列表。
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85)),
                    withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 旁聊里打开附件面板，等相册网格出来，返回第一张照片。主对话被盖住时它的输入栏还在无障碍树里，要上面那间的按钮。
    private func openSideAttach(file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let attaches = app.buttons.matching(identifier: "composerAttach").allElementsBoundByIndex
        tap(attaches.first(where: { $0.isHittable }) ?? attaches[0], file: file, line: line)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons.element(boundBy: 1)
        let first = app.buttons["attachPhoto.0"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        guard first.waitForExistence(timeout: 15) else {
            XCTFail("旁聊的附件面板里没有照片\n\(app.debugDescription)", file: file, line: line)
            return first
        }
        return first
    }

    /// 点主对话里的旁聊卡片进去。翻到卡片后按当时的位置现算坐标：画布里的元素缓存下来的 frame 会过期。
    /// 面板里的“完成”：把选好的照片发出去。
    private func sendPickedPhotos(file: StaticString = #filePath, line: UInt = #line) {
        tap(app.buttons["attachSend"], file: file, line: line)
    }

    /// 等到查询里至少 [count] 个元素出现。
    private func waitForCount(_ query: XCUIElementQuery, _ count: Int, timeout: TimeInterval) -> Bool {
        wait(timeout) { query.count >= count }
    }

    /// 假核心给每张发出去的照片回一条“photo”。
    private var photoEchoes: XCUIElementQuery {
        app.staticTexts.matching(NSPredicate(format: "label == %@", "photo"))
    }

    private func openSideChat(file: StaticString = #filePath, line: UInt = #line) {
        let card = app.buttons["sideChatCard.$ui-fake-5"]
        scrollTo(card, file: file, line: line)
        let f = card.frame
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: f.midX, dy: f.midY)).tap()
        guard wait(10, until: { text(startingWith: "Fixture side ").exists }) else {
            return XCTFail("点了旁聊卡片没进旁聊\n\(app.debugDescription)", file: file, line: line)
        }
    }

    private func tap(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        guard wait(20, until: { element.exists && element.isHittable }) else {
            return XCTFail("找不到或点不到 \(element)\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
