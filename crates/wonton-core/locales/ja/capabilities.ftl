# 機能へのアクセス、依存関係のエラー、インストールの確認。
gateway-capability-permission = アプリ { $id } は機能 { $capability } を宣言していないか、提供元の利用がまだ承認されていません
plugin-capability-missing = 機能 { $capability } の提供元がありません。先に提供元をインストールしてください
plugin-capability-stopped = アプリ { $provider } は機能 { $capability } を提供できる状態ではありません
plugin-capability-credential-failed = アプリ { $provider } で機能 { $capability } のアクセス認証情報の準備または失効に失敗しました（終了コード { $code }）：{ $detail }
plugin-capability-in-use = 次のアプリがこの提供元を必要としています：{ $plugins }。先に別の提供元をインストールするか、これらのアプリをアンインストールしてください。
plugin-capability-provider-install = { $provider }（{ $id }）もインストールします
plugin-capability-provider-permission = { $provider } には次の権限が必要です：{ $permission }
plugin-capability-provider-optional-off = { $provider } の任意の権限はオフのままになります
plugin-capability-connect = { $consumer } は { $provider } を通じて機能 { $capability } を利用できるようになります
plugin-capability-provider-failed = 提供元 { $provider } をインストールできませんでした：{ $error }
plugin-capability-install-selection = 選択した提供元が不足している必須機能と一致しません。インストールのプレビューを開き直し、一覧から提供元を選んでください。
plugin-capability-request-format = この機能設定を読み取れません。有効な JSON リクエストを送信してください。
plugin-capability-model-migration = モデルゲートウェイを任意の機能として宣言するには、より新しいコアが必要です。モデルゲートウェイの移行までは models 権限を使ってください。
plugin-capability-plan-changed = プレビュー後にインストール計画が変わりました。プレビューを開き直し、接続先を確認してください。
plugin-capability-choices-unsupported = このコアでは選択したデバイスや制限付きの Bot 範囲を適用できません。選択内容は変更もインストールもされていません。
plugin-capability-uninstall-selection = 選択した提供元は使用中か、今回の削除候補にありません。アンインストールのプレビューを開き直してください。
