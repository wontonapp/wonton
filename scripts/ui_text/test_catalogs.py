"""分文件规则的用例：键该在哪个文件、重复键、四种语言齐不齐（ADR-0043）。

运行：python3 -m unittest discover -s scripts/ui_text -t scripts（check.sh 和 CI 都跑）
"""

from __future__ import annotations

import unittest

from ui_text import catalogs, groups, paths

IOS_CATALOG = """{
  "sourceLanguage": "en",
  "version": "1.0",
  "strings": {
    "chat.empty": {"localizations": {"en": {"stringUnit": {"state": "translated", "value": "No chats"}}}},
    "store.title": {"localizations": {"en": {"stringUnit": {"state": "translated", "value": "Store"}}}}
  }
}
"""

ANDROID_CATALOG = """<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="chat_empty">No chats</string>
    <string name="store_title">Store</string>
</resources>
"""

FTL_CATALOG = "chat-empty = No chats\nstore-title = Store\n"


class ResolveTest(unittest.TestCase):
    def test_prefix_decides_the_file(self):
        self.assertEqual(groups.resolve("chat.empty", "ios"), ("chat", "chat1"))
        self.assertEqual(groups.resolve("chat_empty", "android"), ("chat", "chat1"))
        self.assertEqual(groups.resolve("chat-empty", "fluent"), ("chat", "chat1"))
        self.assertEqual(groups.resolve("core-error-storage", "fluent"), ("common", "common"))
        self.assertEqual(groups.resolve("Ask a bot: %@", "ios"), ("common", ""))

    def test_emoji_keys_land_by_number(self):
        self.assertEqual(groups.resolve("verification.emoji.0", "ios"), ("peers", "verificationEmoji0"))
        self.assertEqual(groups.resolve("verification.emoji.7", "ios"), ("peers", "verificationEmoji2"))
        self.assertEqual(groups.resolve("verification.accept", "ios"), ("peers", "verification0"))

    def test_unknown_prefix_only_lands_in_common(self):
        group, stem = groups.resolve("futureWidget.title", "ios")
        self.assertEqual((group, stem), ("common", ""))

    def test_file_names_follow_the_platform(self):
        self.assertEqual(groups.file_name("", "ios"), "Localizable.xcstrings")
        self.assertEqual(groups.file_name("chatBackup", "android"), "strings_chat_backup.xml")
        self.assertEqual(groups.file_name("verificationEmoji2", "fluent"), "verification_emoji2.ftl")


class LayoutTest(unittest.TestCase):
    def test_key_in_the_wrong_file_is_reported(self):
        problems = catalogs.layout_problems([("apps/ios/Wonton/strings/chat0.xcstrings", [("store.title", 4)])], "ios")
        self.assertIn("store.xcstrings", problems["apps/ios/Wonton/strings/chat0.xcstrings"][0][1])
        self.assertIn("plugins", problems["apps/ios/Wonton/strings/chat0.xcstrings"][0][1])

    def test_duplicate_key_is_reported(self):
        found = [
            ("apps/ios/Wonton/strings/chat0.xcstrings", [("chat.empty", 3)]),
            ("apps/ios/Wonton/strings/chat1.xcstrings", [("chat.empty", 8)]),
        ]
        problems = catalogs.layout_problems(found, "ios")
        self.assertEqual(problems["apps/ios/Wonton/strings/chat1.xcstrings"][0][0], 8)

    def test_same_key_in_two_languages_is_no_duplicate(self):
        stem = groups.resolve("chat_empty", "android")[1]
        found = [
            (paths.android_path(paths.ANDROID_MODULES[0], "en", stem), [("chat_empty", 3)]),
            (paths.android_path(paths.ANDROID_MODULES[0], "ja", stem), [("chat_empty", 3)]),
        ]
        self.assertEqual(catalogs.layout_problems(found, "android"), {})


class AndroidPairsTest(unittest.TestCase):
    def test_missing_translation_is_reported_on_the_en_file(self):
        problems = catalogs.android_problems(ANDROID_CATALOG, {"ja": None, "zh-Hans": ANDROID_CATALOG, "zh-Hant": ANDROID_CATALOG})
        self.assertIn("values-ja/strings.xml", problems["en"][0][1])

    def test_missing_en_file_is_reported(self):
        problems = catalogs.android_problems("", {"ja": ANDROID_CATALOG, "zh-Hans": None, "zh-Hant": None})
        self.assertIn("英文的 values 里没有这份文件", problems["en"][0][1])


class FtlPairsTest(unittest.TestCase):
    def test_missing_language_file_is_reported_on_the_en_file(self):
        problems = catalogs.ftl_missing("chat0", {"en": FTL_CATALOG, "zh-Hans": FTL_CATALOG, "zh-Hant": None, "ja": FTL_CATALOG})
        self.assertIn("zh-Hant/chat0.ftl", problems[paths.ftl_path("en", "chat0")][0][1])

    def test_key_only_in_one_language_is_reported(self):
        problems = catalogs.ftl_missing("chat0", {"en": FTL_CATALOG, "zh-Hans": FTL_CATALOG, "zh-Hant": FTL_CATALOG, "ja": "chat-empty = No chats\nchat-only = Just ja\n"})
        self.assertIn("「chat-only」在 en 里没有", [message for _, message in problems[paths.ftl_path("ja", "chat0")]][1])

    def test_missing_key_is_reported_on_that_language(self):
        problems = catalogs.ftl_missing("chat0", {"en": FTL_CATALOG, "zh-Hans": "chat-empty = 没有会话\n", "zh-Hant": FTL_CATALOG, "ja": FTL_CATALOG})
        self.assertIn("缺少「store-title」", problems[paths.ftl_path("zh-Hans", "chat0")][0][1])


class ParsersTest(unittest.TestCase):
    def test_xcstring_keys(self):
        self.assertEqual(catalogs.xcstring_keys(IOS_CATALOG), [("chat.empty", 5), ("store.title", 6)])

    def test_android_keys(self):
        self.assertEqual(catalogs.android_keys(ANDROID_CATALOG), [("chat_empty", 3), ("store_title", 4)])

    def test_ftl_keys(self):
        self.assertEqual(catalogs.ftl_keys(FTL_CATALOG), [("chat-empty", 1), ("store-title", 2)])


if __name__ == "__main__":
    unittest.main()
