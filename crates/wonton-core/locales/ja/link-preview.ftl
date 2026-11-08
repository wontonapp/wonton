## リンクプレビューのエラー（ADR-0065）。App はカテゴリーごとに自分の文言を出し、ここの文は「詳細」に入ります。
## { $detail } は生の値です：アドレス、ステータス、コンテンツタイプ、または下層のエラー。

link-preview-invalid-url = プレビューできるのは、ポート 80・443・8080・8443 の http と https のリンクだけです：{ $detail }
link-preview-blocked-address = リンク先がローカルまたはプライベートのアドレスなので、自宅サーバーは開きません：{ $detail }
link-preview-dns-failed = プロキシ経由でドメインを解決できず、サイトに接続できませんでした：{ $detail }
link-preview-upstream-error = サイトに接続できないか、エラーが返されました：{ $detail }
link-preview-timeout = サイトから { $detail } 以内に応答がありませんでした
link-preview-not-html = リンク先はWebページではありません：{ $detail }
link-preview-no-preview = ページにタイトル・説明・画像がありません：{ $detail }
link-preview-too-many-requests = このデバイスからのプレビュー取得が多すぎます。1 分後にもう一度お試しください
link-preview-image-gone = プレビュー画像の期限が切れました。プレビューをもう一度取得してください
