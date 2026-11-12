## ブリッジ自身がルームに送る状態文（ADR-0043）。1つの文を1つのメッセージにし、
## データは引数として埋め込みます（翻訳しない）。です・ます体。

login-reconnect = ログインしました。再接続します。
input-unsupported =
    対応していない入力です：{ $kind ->
        [audio] 音声メッセージ
        [video] 動画メッセージ
        *[other] この種類のコンテンツ
    }はまだ対応していません。テキスト・画像・ファイルのみ送れます。
input-too-long = 入力が正しくありません：入力が { $max } バイトの上限を超えています
input-empty = 入力が正しくありません：入力は空にできません
attachment-too-large = 添付ファイルが { $max } の上限を超えています
workspace-missing = ワークスペースのディレクトリがありません。プラグインがマウントしていない可能性があります
workspace-not-absolute = ワークスペースのパスが絶対パスではありません：{ $path }
control-without-text = 入力が正しくありません：制御コマンドにテキストがありません
interrupt-pending = 取り込み中：すでに割り込み要求が待機しています。
queue-full = 取り込み中：ACP のキューが満杯です。しばらくしてからお試しください。
no-active-session = エージェントの準備ができていません：実行中の ACP セッションがありません
steer-rejected = エージェントは割り込みを受け付けませんでした。
session-lost-new = ACP セッションが失われたため、新しいセッションを作成しました。以前の文脈は使えません。
permission-failed = エージェントの権限リクエストに失敗しました：{ $message }
permissions-cancelled = 権限リクエストを取り消しました。エージェントはその操作を実行していません。
output-limited = 上流のエラー：エージェントの応答が出力上限を超えたため、このターンを取り消しました。
no-text = エージェントはこのターンでテキストを出力しませんでした（stopReason={ $stop_reason }）。
transport-disconnected = 通信できません：ACP エージェントとの接続が切れたため、まもなく再接続します。

# ログにだけ出る文（ログは翻訳しない。種別の接頭辞とつながって読む）。
agent-connect-socket = ACP ソケット { $path } に接続できません：{ $error }
agent-init-timeout = ACP initialize がタイムアウトしました
direct-task-failed = ダイレクトメッセージの処理に失敗しました：{ $error }
stream-closed = Matrix ルームのメッセージストリームが閉じました
catch-up-dropped =
    ただいまオフラインでした。次の { $count } 件は処理していません：
    { $list }
catch-up-dropped-more =
    ただいまオフラインでした。次の { $count } 件は処理していません：
    { $list }
    ……ほか { $rest } 件は省略しています

