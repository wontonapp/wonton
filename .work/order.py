import os
B = os.environ.get("BASE", "192.168.0.69.nip.io")
PORT = os.environ.get("PORT", "19680")
ORDER = [
    "https://example.com",
    "example.com/a",
    f"http://app-whoami.{B}:{PORT}/",
    f"https://matrix.to/#/@family:{B}",
    "mailto:test@example.com",
    "test@example.com",
    "+86 138 0013 8000",
    "代码：\n```\nfn main() {\n    println!(\"hi\");\n}\n```",
    "长消息：" + "这是一条用来检查折行和回到最新按钮的长消息，混着链接 https://example.com/long 。" * 8,
]
