## トピックセッション（ADR-0058）でチャットに送る文と、エージェントに組み立てる文。

name-main = メイン会話

topic-no-active-session = 有効な ACP セッションがありません
topic-input-too-long = 入力が { $size } バイトの上限を超えています
topic-input-empty = 入力は空にできません
topic-control-no-text = 制御コマンドにテキストがありません
topic-session-lost = ACP セッションが失われたので新しいセッションを作成しました。以前のコンテキストは使えません。
topic-queued = トピックをキューに入れました。現在 { $ahead } 件のトピックが動作中で、終わったら自動で開始します。
topic-permission-failed = エージェントの権限リクエストに失敗しました: { $message }
topic-permissions-cancelled = 権限リクエストをキャンセルしました。エージェントは操作を実行していません。
topic-output-limited = 上流エラー: エージェントの返信が出力上限を超えたため、このターンを取り消しました。
topic-no-text = エージェントはこのターンでテキストを出力しませんでした（stopReason={ $reason }）。
topic-idle-ended = トピックが30分以上アイドルだったため終了しました。
topic-main-cannot-end = メイン会話は終了できません。
topic-already-ended = このトピックはすでに終了しました。
topic-ended-running = トピックは終了しました。このターンはまだ実行中で、結果はこのトピックに送ります。
topic-ended = トピックを終了しました。
topic-ended-idle = 30分以上アイドル
topic-ended-manual = ユーザーが手動で終了

status-working = 作業中
status-waiting = 確認待ち
status-idle = アイドル
status-finished = 終了
status-queued = キュー待ち

conflict-reject-reason = 「{ $other }」が { $path } を編集中で、ユーザーは今は変更しないことを選びました
conflict-reject-reason-short = ユーザーは変更しないことを選びました
conflict-approved = 「{ $name }」がこのターンを終えました。{ $path } の変更は承認されました。
conflict-in-use-title = { $title }（「{ $other }」も編集中）
conflict-wait-option = 「{ $other }」が終わるまで待つ
conflict-now-option = 今すぐ変更する
conflict-reject-option = 変更しない
conflict-in-use-note = 「{ $other }」が { $path } を編集中です。そのターンが終わる前に変更すると上書きされるおそれがあります。

digest-header = 他のセッションの動き:
digest-working = 「{ $name }」が実行中: { $doing }
digest-waiting = 「{ $name }」が操作の確認を待っています
digest-queued = 「{ $name }」はキュー待ちです
digest-intro = これはメイン会話から分かれたトピック「{ $name }」です。
digest-preceding-header = メイン会話の抜粋（参考用）:
digest-board-header = 現在のボード:
digest-no-others = 他のセッションはありません
digest-rules-line = 役割の約束: { $rules }
digest-rules = このトピックでユーザーが頼んだことだけを担当する。メイン会話や他のトピックがやっていることを続けない。それらが編集中のファイルに触れるときは、先に止まってユーザーに聞く。
digest-first = ユーザーの最初の文: { $root }
digest-done = トピック「{ $name }」: 完了
digest-done-files = トピック「{ $name }」: 完了、{ $files } ファイル変更
digest-done-own = 「{ $name }」が完了しました
digest-done-summary = 「{ $name }」が完了しました: { $summary }
digest-files-count = 、{ $files } ファイル変更
digest-files-list = 、変更: { $files }
digest-executed = 「{ $name }」が「{ $title }」を実行しました
digest-conflict = 「{ $first }」と「{ $second }」がどちらも { $path } を変更しました（後から判明）
digest-ended = 「{ $name }」が終了しました（{ $reason }）
digest-board-line = 「{ $name }」{ $status }
digest-board-line-doing = 「{ $name }」{ $status }: { $doing }
topic-exec-default = コマンド
topic-no-ask-mode = このエージェントには変更前に確認するモードがありません（WONTON_ACP_TOPIC_MODE が未設定か、エージェントが対応していません）。ファイル変更前に確認しないため、衝突は後からしか分かりません。
