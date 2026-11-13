## 自宅サーバーがチャットに送るテキスト（ADR-0043）：確認カードとその結果行、他の
## Matrix クライアント向けのプレーンテキストカード、Bot のルームを引き渡せないときの
## キック理由、エージェントがあなたに伝えるヘルプのテキスト。1文を1メッセージに。
## データ（Bot 名、アプリ名、バージョン、パス、上游のエラー原文）はパラメーターとして
## 文に埋め込み、翻訳しない。

# 確認カードのボタン。
chat-choice-install = インストール
chat-choice-deny-install = インストールしない
chat-choice-update = アップデート
chat-choice-deny-update = アップデートしない
chat-choice-uninstall = アンインストール
chat-choice-cancel = キャンセル
chat-choice-allow = 許可
chat-choice-deny = 拒否

# ツール確認カードの許可期間オプション：一定時間、再確認なしで呼び出せます。パラメーターは
# 分数または秒数の整数。
chat-card-window-minutes = 次の{ $minutes }分間は許可
chat-card-window-seconds = 次の{ $secs }秒間は許可

# インストールとアップデートのカードの行。$requester は Bot の名前、$title はアプリ名。
chat-card-image = サービス{ $service }はイメージ{ $image }を使用
chat-card-ports-added = 追加されるローカルネットワークのポート：{ $ports }
chat-card-gpu-added = 追加されるGPUサービス：{ $services }
chat-card-version-official = バージョン{ $version }（公式インデックスから）
chat-card-version = バージョン{ $version }
chat-card-update-from = バージョン{ $from }から{ $to }へのアップデート
chat-card-no-permissions = 追加の権限は不要
chat-card-permissions-header = 付与する権限：
chat-card-agent-source = あなたのエージェントから（{ $requester }）・未審査：コードはエージェントが書いたものです。内容はあなたが確認してください
chat-card-agent-ask-again = 今後{ $requester }がこのアプリを更新するときは確認しません（権限が増えるときは確認します）
chat-card-update-header-added = このアップデートで追加される項目：
chat-card-update-header-risky = このアップデートは危険度の高い権限を要求しています：
chat-card-update-header-ask-every = 権限は変わっていませんが、「毎回確認」が有効になっています：
chat-card-update-header-adopt = このアプリはあなたがインストールしたものです。承認すると { $requester } が管理します：
chat-card-update-adopt-replace = 承認するとインストール済みの版を置き換えます。データは保持され、失敗時は元に戻ります

# カードのタイトル。$requester は Bot の名前、$title はアプリが渡す名前。
chat-card-install-title = { $requester }が{ $title }のインストールを要求しています
chat-card-update-title = { $requester }が{ $title }のアップデートを要求しています
chat-card-uninstall-title = { $requester }が{ $title }のアンインストールを要求しています
chat-card-permission-enable-title = { $requester }が「{ $title }」の{ $permission }をオンにしようとしています
chat-card-permission-disable-title = { $requester }が「{ $title }」の{ $permission }をオフにしようとしています
chat-card-permission-off-loss = 失うもの：{ $loss }
chat-card-permission-root = オンにするとこのアプリはホスト全体を操作でき、root で実行されます。
chat-choice-permission-enable = オンにする
chat-choice-permission-disable = オフにする
chat-permission-done = 「{ $title }」の権限を変更しました
chat-permission-failed = { $id } の権限の変更に失敗しました：{ $error }

# エージェント権限カードのタイトル。$title はエージェントが付けたリクエスト名。
chat-card-agent-title = { $requester }：{ $title }

# 最高レベルの権限で追加で押すカード（ADR-0036）。
chat-card-highest-title = { $title }はホスト全体を制御できます
chat-card-highest-line = コードはこのホストのroot権限で実行されます
chat-card-highest-confirm = このアプリがホスト全体を制御することを承知しました

# アンインストールのカード：アプリのデータの扱い。
chat-card-uninstall-purge = データも削除します。ボットとの過去の暗号化メッセージも読めなくなり、元に戻せません
chat-card-uninstall-keep = データは残します。次にインストールしたときにも使えます

# エージェント権限カードの引数。$args は生の JSON で、長いときは切り詰めます。
chat-card-args = 引数：{ $args }

# Bot のファイル保存カード。$name と $size は1つのファイル、$total は合計サイズ。
chat-card-save-file = { $name }（{ $size }）
chat-card-save-more = ほか{ $count }件
chat-card-save-total = 合計{ $total }
chat-card-save-unreviewed = ボットが提供したファイルです。内容は未審査です
chat-card-save-allow = { $folder }への保存を許可
chat-card-save-title = { $requester }が{ $count }件のファイルを{ $folder }に保存しようとしています

# 他の Matrix クライアント向けのカード本文。$message は回答時に書かれた結果行で、
# あなたの言語です。
chat-card-state-pending = Wontonアプリで確認してください。
chat-card-state-running = 実行中…
chat-card-state-succeeded = 完了：{ $message }
chat-card-state-failed = 実行できませんでした：{ $message }
chat-card-state-uncertain = 結果を確認できません：{ $message }
chat-card-state-rejected = 拒否したため実行されていません
chat-card-state-expired = 期限切れのため実行されていません

# カード本文の下の許可期間の行：有効中、または終了した経緯。$count は最初の1回を除いて
# 期間内に許可された回数。0 は不明または 0 回。
chat-window-active = 許可期間中です。期間が終わるまで確認はありません。
chat-window-ended =
    { $reason ->
        [expired]
            { $count ->
                [0] 許可期間が終了しました。
               *[other] 許可期間が終了しました（{ $count }回許可）。
            }
        [stopped]
            { $count ->
                [0] 許可期間を停止しました。
               *[other] 許可期間を停止しました（{ $count }回許可）。
            }
        [restarted] サーバーが再起動したため、許可期間は終了しました。
       *[other]
            { $count ->
                [0] 許可期間が終了しました。
               *[other] 許可期間が終了しました（{ $count }回許可）。
            }
    }

# 回答の後にカードに書かれる結果行。
chat-install-done = { $title }をインストールしました
chat-install-done-url = { $title }をインストールしました。アドレス：{ $url }
chat-install-failed = インストールできません（{ $kind }）：{ $error }
chat-install-failed-id = { $id }をインストールできません（{ $kind }）：{ $error }
chat-install-note-writer = アプリの作成者を記録できませんでした（{ $error }）。次回のアップデートが失敗します
chat-outcome-with-note = { $message }。{ $note }
chat-update-done = { $version }にアップデートしました
chat-update-done-url = { $version }にアップデートしました。アドレス：{ $url }
chat-update-failed = { $id }をアップデートできません（{ $kind }）：{ $error }
chat-uninstall-done-purge = { $title }をアンインストールしました。データは削除されました
chat-uninstall-done-keep = { $title }をアンインストールしました。データは残っています
chat-uninstall-failed = { $id }をアンインストールできません（{ $kind }）：{ $error }
chat-tool-ran = 実行しました
chat-tool-failed = ツールが失敗を報告しました
chat-save-done = { $folder }に保存しました
chat-save-done-note = { $folder }に保存しました。{ $note }
chat-save-partial = { $saved }を保存しました。残りは失敗しました：{ $failure }
chat-save-note-grant = 権限を記録できませんでした（{ $error }）。次回も確認を求めます
chat-agent-chose = 「{ $choice }」を選択しました
chat-agent-abandoned = エージェントが待つのをやめたため、この確認は無効になりました
chat-superseded = 新しいリクエストに置き換えられました
chat-uncertain-restart = サーバーが再起動したため、完了したかどうか不明です

# キック理由：Bot のルームを引き渡せなかったとき、ルームの timeline に表示されます。
chat-kick-reason = Wontonがこのチャットを作成できませんでした

# ヘルプツール。エージェントはこれを読んであなたに答えます。$details は上游の原因、
# $required と $core はバージョン番号。
chat-help-docs-unavailable = このサーバーにはヘルプがありません（{ $dir }になく、取得したこともありません）
chat-help-refresh-failed = 新しいヘルプを取得できませんでした：{ $details }
chat-help-refresh-needs-core = 新しいヘルプがありますが、サーバーを{ $required }以上にアップデートする必要があります（このサーバーは{ $core }）。先にサーバーをアップデートしてください
chat-help-docs-age = { $problem }。これは{ $days }日前のヘルプです
chat-help-topic-missing = トピック「{ $topic }」はありません。選べるトピック：{ $topics }
chat-help-page-needs-core = このページの機能を使うには、サーバーを{ $required }以上にアップデートする必要があります（このサーバーは{ $core }）。先にサーバーをアップデートしてください

chat-mail-convert-failed = メールの HTML 本文をテキストに変換できませんでした。元の HTML を message.html として添付しています。
chat-mail-unknown-source = 送信元が不明、またはメール送信権限のないアプリからのメールを受信しました。誰にも配信していません。
chat-mail-recipients-failed = アプリのメール受信者を更新できませんでした：{ $error }。家族の変更時かサーバーの再起動時に再試行します。
chat-notify-rate-limited = { $plugin } の通知とメールが多すぎるため、一時停止しました。約{ $seconds }秒後に自動で再開します。
# Bot がパネルにページをピン留めする（ADR-0052）。
panel-pin-card-title = { $bot } がパネルにページを置こうとしています
panel-pin-card-line = 許可すると、この Bot は以後メインパネルに確認なしでページを追加できます。設定で取り消せます。
panel-pin-allowed = 許可しました
panel-pin-bot-only = ページをピン留めできるのは Bot だけです。プラグイン { $plugin } に Bot の身元がありません
panel-pin-bot-missing = Bot { $bot } の記録がありません
panel-pin-full = パネルは満杯です（最大 { $max } ページ）。先にページを外してください
panel-pin-already-asking = メインパネルへの確認カードがすでに回答待ちです
panel-pin-matrix = Matrix に接続できません：{ $error }
panel-pin-not-there = そのページはパネルにピン留めされていません

# MCP Apps 界面里的调用在卡片上把调用方写成主人。
mcp-app-owner = あなた

# 跳数到顶（ADR-0045）：几个 Bot 在一个房间里来回说得太多，网关停下了。
chat-card-hop-limit-title = { $bots }が{ $hops }回やり取りしたため、停止しました
chat-choice-hop-limit-continue = 続ける
chat-choice-hop-limit-stay = 止めておく
chat-hop-limit-resumed = 再開しました
chat-hop-limit-kept-by-other = 相手が止めておくを選んだため、停止したままです

# Chain limit card (ADR-0061 §10).
chat-chain-limit-title = この呼び出しの連鎖が上限に達しました
chat-chain-limit-title-rounds = この連鎖は往復の上限に達しました
chat-chain-limit-title-cost = この連鎖は利用量の上限に達しました
chat-chain-limit-bots = この連鎖のボット：{ $bots }
chat-chain-limit-what = 「もう 1 回分」で往復と利用量の上限を 1 回分ずつ増やします。「止めておく」で連鎖ごと止まり、続けるにはもう一度話しかけてください。
chat-chain-limit-grant = もう 1 回分
chat-chain-limit-stop = 止めておく
chat-chain-stopped-card = 連鎖が止まったため、このカードは無効です

# メッセージ送信カード（ADR-0061 §9）：$recipient は受信者、$bot は送信するボット。
chat-send-message-title = { $recipient } にメッセージを送る
chat-send-message-queued = 承認済み。{ $bot } の送信を待っています

chat-handoff-title = { $from } → { $bot }
chat-handoff-risk = このボットは対象の実行環境でコマンドの実行、ファイルの変更、ログイン情報や別の会話を含むすべての読み取りができます。
chat-handoff-started = 引き継ぎました。対象のボットが処理中です
