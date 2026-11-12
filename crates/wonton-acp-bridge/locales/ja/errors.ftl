## ブリッジがチャットに送るエラー。種別の接頭辞が先に来て、詳細（上流の原文、IO エラー）は
## 引数として文に埋め込みます（翻訳しない）。接頭辞と次のブリッジ自身の詳細文には
## 文末の句点を付けません（詳細が接頭辞の後ろに続くため）。

err-auth-required = エージェントのログインが必要です：/wonton login でログイン方法を確認できます。
err-not-ready = エージェントの準備ができていません：{ $message }
err-transport = 通信できません：{ $message }
err-session = ACP セッションを利用できません：{ $message }
err-upstream = 上流のエラー：{ $message }
err-invalid = 入力が正しくありません：{ $message }
err-confirmation = 確認カードを利用できません：{ $message }
err-busy = 取り込み中：{ $message }
err-matrix = Matrix のエラー：{ $message }
err-hop-limit = 複数の Bot がここで何度もやり取りしたため、ゲートウェイが私のメッセージ { $count } 件を止めていました。今送り直しました。

# ブリッジ自身が書く詳細。上の接頭辞の後ろに入ります。
acp-no-http-mcp = エージェントが HTTP MCP に対応していません
acp-no-session-id = session/new が sessionId を返しません
acp-no-steering = エージェントが _session/steering に対応していません
acp-connection-closed = ACP 接続が閉じました
acp-turn-timeout = ACP の1ターンがタイムアウトしたため、キャンセルを要求しました
acp-output-limit = ACP の応答が出力上限を超えたため、キャンセルを要求しました
acp-encode-failed = ACP リクエストをエンコードできませんでした：{ $error }
acp-request-failed = ACP リクエストに失敗しました
acp-frame-too-large = ACP リクエストが1フレームの上限を超えました
acp-restore-unsupported = エージェントは ACP セッションの復元に対応していません

# 確認カード（Bot の権限確認）。
perm-mcp-url = WONTON_MCP_URL が正しくありません：{ $error }
perm-client = 確認カード用の HTTP クライアントを作成できませんでした：{ $error }
perm-broker-connect = 確認カードのエンドポイントに接続できません：{ $error }
perm-broker-read = 確認カードの応答を読み取れませんでした：{ $error }
perm-broker-timeout = 確認カードのリクエストがタイムアウトしました
perm-broker-invalid = 確認カードの応答が無効です：{ $error }
perm-tool-kind = ACP ツール種別：{ $kind }
perm-missing-options = ACP 権限リクエストに options がありません
perm-duplicate-option-id = ACP 権限リクエストの optionId が重複しています
perm-invalid-kind = ACP 権限オプションの kind が無効です：{ $error }
perm-empty-options = ACP 権限リクエストに options が1つもありません
perm-missing-field = ACP 権限リクエストに { $field } がありません
perm-redacted = [非表示]

# 呼び出しチェーン（ADR-0061 第 10 節）：コアがチェーンを返さないとき、このターンは Agent に送らず、部屋で理由を伝える。
chain-limit-depth = この会話チェーンは Agent の層数が上限に達したため、このターンは開始できません。続けるには Wonton でこのチェーンに「追加を許可」してください。
chain-limit-rounds = この会話チェーンはターン数を使い切ったため、このターンは開始できません。続けるには Wonton でこのチェーンに「追加を許可」してください。
chain-limit-cost = この会話チェーンは費用が上限に達したため、このターンは開始できません。続けるには Wonton でこのチェーンに「追加を許可」してください。
chain-limit-other = この会話チェーンは上限に達したため、このターンは開始できません。続けるには Wonton でこのチェーンに「追加を許可」してください。
chain-stopped = この会話チェーンは停止済みのため、このターンは開始できません。続けるには新しいメッセージとしてもう一度送ってください。
err-chain-refused = 呼び出しチェーンがこのターンを拒否しました：{ $message }
chain-unreachable = 呼び出しチェーンの接続に失敗しました：{ $error }
chain-invalid = 呼び出しチェーンの応答が無効です（HTTP { $status }）：{ $error }
chain-client = 呼び出しチェーンの HTTP クライアントの作成に失敗しました：{ $error }
chain-url = WONTON_MCP_URL が無効です：{ $error }
outbox-client = 送信箱の HTTP クライアントの作成に失敗しました：{ $error }
