"""给主人的提示和旁聊发回主时间线的文字，四种语言各一条。

入口照 App 里实际显示的写：「应用 → <插件标题>」打开 Hermes 的网页，Keys、Models 是网页侧栏的名字。清单的标题只有中文和
英文，繁中界面显示简体标题，日文界面显示英文标题；Hermes 网页默认英文，所以页名都写英文。和清单的 title 一起改。

旁聊的引号沿用 ACP 桥的话题文字（四种语言都用「」，见 wonton-acp-bridge 的 locales/*/topics.ftl）。
"""
NO_MODEL = {
    "en": "No model is set up yet: open Apps → All-Purpose Bot (Hermes), add an AI provider under Keys (fill in its "
    "API key or sign in), pick a model under Models, then message me again.",
    "zh-Hans": "还没有设置模型：请打开「应用 → AI 管家（Hermes）」，在 Keys 页新加一个模型服务商、填它的 API key 或登录订阅，"
    "再在 Models 页选一个模型，然后再给我发消息。",
    "zh-Hant": "還沒有設定模型：請打開「應用程式 → AI 管家（Hermes）」，在 Keys 頁新增一個模型供應商、填入它的 API 金鑰或登入訂閱，"
    "再在 Models 頁選一個模型，然後再傳訊息給我。",
    "ja": "モデルがまだ設定されていません。「アプリ → All-Purpose Bot (Hermes)」を開き、Keys で AI サービスを新しく追加し"
    "（API キーを入力するかサブスクリプションでログイン）、Models でモデルを選んでから、もう一度送ってください。",
}

# 旁聊结果行的 body（旁聊约定的样例：旁聊「标题」：已完成）。state 用英文码进结构化字段，
# body 里的说法用主人的语言。
SIDE_CHAT_BODY = {
    "en": "Side chat 「{title}」: {state}",
    "zh-Hans": "旁聊「{title}」：{state}",
    "zh-Hant": "旁聊「{title}」：{state}",
    "ja": "サイドチャット「{title}」：{state}",
}

SIDE_CHAT_STATE = {
    "done": {"en": "done", "zh-Hans": "已完成", "zh-Hant": "已完成", "ja": "完了"},
    "failed": {"en": "failed", "zh-Hans": "失败", "zh-Hant": "失敗", "ja": "失敗"},
    "ended": {"en": "ended", "zh-Hans": "已结束", "zh-Hant": "已結束", "ja": "終了"},
}

# 主会话下一轮注入的旁聊动态，格式照 ACP 桥的话题动态（标题行 + 「- 」条目）。
SIDE_CHAT_HEADER = {
    "en": "Side chat updates:",
    "zh-Hans": "旁聊的动态：",
    "zh-Hant": "旁聊的動態：",
    "ja": "サイドチャットの動き:",
}

SIDE_CHAT_DIGEST = {
    "en": "Side chat 「{title}」: {state}. {excerpt}",
    "zh-Hans": "旁聊「{title}」{state}：{excerpt}",
    "zh-Hant": "旁聊「{title}」{state}：{excerpt}",
    "ja": "サイドチャット「{title}」{state}：{excerpt}",
}

# 没有摘录（Bot 重启过、或那轮没有回复）时只报标题和状态。
SIDE_CHAT_DIGEST_BRIEF = {
    "en": "Side chat 「{title}」: {state}",
    "zh-Hans": "旁聊「{title}」{state}",
    "zh-Hant": "旁聊「{title}」{state}",
    "ja": "サイドチャット「{title}」{state}",
}

# App 一次发的附件组没到齐（等了 120 秒）就开的一轮：这一轮文字末尾的说明，
# 和发给主人的 m.notice。之后才到的会单独处理，不再等。
BATCH_PARTIAL = {
    "en": "(This batch has {total} messages; only {received} arrived.)",
    "zh-Hans": "（这一组 {total} 条只收到 {received} 条）",
    "zh-Hant": "（這一組 {total} 條只收到 {received} 條）",
    "ja": "（この {total} 件のうち {received} 件しか届いていません）",
}

BATCH_NOTICE = {
    "en": "Only {received} of {total} messages from your batch arrived, so I went ahead with those; anything else will be handled on its own.",
    "zh-Hans": "这一组 {total} 条消息只收到了 {received} 条，先按收到的处理了；之后再到的会单独处理。",
    "zh-Hant": "這一組 {total} 條訊息只收到了 {received} 條，先按收到的處理了；之後再到的會單獨處理。",
    "ja": "{total} 件のうち {received} 件しか届いていないため、届いた分だけ処理しました。後から届いたものは別々に扱います。",
}

# 不接这一轮时回给说话人的话（调用链被核心拒掉，这一轮不推理）。
CHAIN_STOPPED = {
    "en": "This chain was stopped. Send the message again to start a new one.",
    "zh-Hans": "这条链已经停了，重新发一句就开一条新链。",
    "zh-Hant": "這條鏈已經停了，重新傳一句就開一條新鏈。",
    "ja": "このチェーンは停止しました。もう一度送ると新しいチェーンが始まります。",
}

CHAIN_LIMIT = {
    "en": "This chain is at its limit. Its owner has to extend it before it can go on.",
    "zh-Hans": "这条链到上限了，要等链代表的人同意加额度才能接着做。",
    "zh-Hant": "這條鏈到上限了，要等鏈代表的人同意加額度才能接著做。",
    "ja": "このチェーンは上限に達しました。続けるにはチェーンの代表者の承認が必要です。",
}

CHAIN_DENIED = {
    "en": "The core doesn't know the sender as someone this bot listens to.",
    "zh-Hans": "核心不认这个说话的人，他不在这个 Bot 听的人里。",
    "zh-Hant": "核心不認這個說話的人，他不在這個 Bot 聽的人裡。",
    "ja": "発言者をこの Bot の聞く相手としてコアが認めていません。",
}

CHAIN_SELF = {
    "en": "The message came back to the same bot; a chain needs another bot.",
    "zh-Hans": "消息又派回这个 Bot 自己了，链上要换一个 Bot。",
    "zh-Hant": "訊息又派回這個 Bot 自己了，鏈上要換一個 Bot。",
    "ja": "メッセージが同じ Bot に戻ってきました。チェーンには別の Bot が必要です。",
}

CHAIN_LOOP = {
    "en": "This bot is already further up the chain, so the message needs a new chain.",
    "zh-Hans": "这个 Bot 已经在这条链的上级，接着做要另起一条链。",
    "zh-Hant": "這個 Bot 已經在這條鏈的上級，接著做要另起一條鏈。",
    "ja": "この Bot はすでにチェーンの上流にいます。続けるには新しいチェーンが必要です。",
}

CHAIN_OTHER = {
    "en": "The core didn't accept this turn.",
    "zh-Hans": "核心这次没收下这一轮。",
    "zh-Hant": "核心這次沒收下這一輪。",
    "ja": "コアは今回のターンを受け付けませんでした。",
}
