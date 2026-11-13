"""规则一、规则二的行为用例：给一段源码，断言抓到几处、在哪一行，不断言提示文案。

运行：python3 -m unittest discover -s scripts/ui_text -t scripts（check.sh 和 CI 都跑）
"""

from __future__ import annotations

import unittest

from ui_text.components import kotlin_components, swift_components
from ui_text.source import scan_source, source_lang

KOTLIN_DESIGN = """
@Composable
fun WontonRow(
    title: String,
    value: String? = null,
) {}

@Composable
fun WontonIconButton(icon: ImageVector, label: String, onClick: () -> Unit) {}

class WontonSheetAction(
    val title: String,
    val icon: ImageVector,
)
"""

SWIFT_DESIGN = """
struct WontonRow: View {
    let title: String
}

private struct WontonSheetView<Content: View>: View {}

extension View {
    func wontonPushedScreen(_ title: String, showsStatusLine: Bool = false) -> some View { self }
}
"""


def lines(text: str, lang: str, components=()) -> list:
    return [line for line, _ in scan_source(text, lang, components)]


class ComponentDiscovery(unittest.TestCase):
    def test_kotlin_components_take_text_first(self):
        names = {name for _, name in kotlin_components(KOTLIN_DESIGN, design_dir=True)}
        self.assertEqual(names, {"WontonRow", "WontonSheetAction"})

    def test_kotlin_outside_design_dir_needs_wonton_prefix(self):
        found = kotlin_components("fun ProbeRow(title: String) {}\nfun WontonBadge(text: String) {}", design_dir=False)
        self.assertEqual({name for _, name in found}, {"WontonBadge"})

    def test_swift_views_and_modifiers(self):
        found = swift_components(SWIFT_DESIGN, design_dir=True)
        self.assertEqual(found, {("调用", "WontonRow"), ("修饰", "wontonPushedScreen")})


class KotlinRules(unittest.TestCase):
    components = kotlin_components(KOTLIN_DESIGN, design_dir=True)

    def test_design_component_first_argument(self):
        src = 'WontonRow("Test speed")\nWontonRow(stringResource(R.string.speed))\n'
        self.assertEqual(lines(src, "kotlin", self.components), [1])

    def test_text_named_arguments(self):
        src = (
            'WontonDialog(title = "Delete?", message = "This cannot be undone")\n'
            'Probe(subtitle = "Fast", detail = "Mirror reachable")\n'
            'Icon(contentDescription = "Back")\n'
        )
        self.assertEqual(lines(src, "kotlin"), [1, 1, 2, 2, 3])

    def test_not_text_values(self):
        src = (
            'Field(placeholder = "https://api.example.com/v1")\n'
            'Field(placeholder = "mirror.example.com:5000")\n'
            'Box(label = "skeleton")\n'
            'Text("%1$s · %2$s")\n'
            'Text("${count}")\n'
        )
        self.assertEqual(lines(src, "kotlin"), [])

    def test_animation_label_is_not_text(self):
        src = 'val alpha by animateFloatAsState(1f, label = "Fade In")\n'
        self.assertEqual(lines(src, "kotlin"), [])

    def test_legacy_picker_call(self):
        src = 'fun tabText(zh: String, en: String): String = en\nval back = tabText(a, b)\n'
        self.assertEqual(lines(src, "kotlin"), [2])


class SwiftRules(unittest.TestCase):
    components = swift_components(SWIFT_DESIGN, design_dir=True)

    def test_design_component_and_modifier(self):
        src = 'WontonRow("Test speed")\nContent().wontonPushedScreen("Mirrors")\nWontonRow(title: model.title)\n'
        self.assertEqual(lines(src, "swift", self.components), [1, 2])

    def test_text_labels_on_custom_views(self):
        src = (
            'Problem(title: "Mirror wasn\'t saved", detail: error.message)\n'
            'RegistryProbeRow(title: "Docker Hub", probe: result)\n'
            'Banner(message: "Saved", prompt: "Try again")\n'
        )
        self.assertEqual(lines(src, "swift"), [1, 2, 3, 3])

    def test_labels_outside_calls_are_ignored(self):
        src = 'func show(title: String = "Default") {}\nlet value = flag ? title : "Other"\n'
        self.assertEqual(lines(src, "swift"), [])

    def test_system_controls(self):
        src = (
            'Toggle("Show labels", isOn: $on)\n'
            '.navigationTitle("Settings")\n'
            '.alert("Delete?", isPresented: $shown) {}\n'
            'Label("Retry", systemImage: "arrow.clockwise")\n'
        )
        self.assertEqual(lines(src, "swift"), [1, 2, 3, 4])

    def test_symbol_names_are_not_text(self):
        src = 'Image(systemName: "chevron.left")\nWontonIconButton(icon: .gear, label: model.name)\n'
        self.assertEqual(lines(src, "swift"), [])

    def test_legacy_picker_call(self):
        src = 'Text(TabText.choose("a", "b"))\nlet x = choose("a", "b")\n'
        self.assertEqual(lines(src, "swift"), [1])


class RustRules(unittest.TestCase):
    def test_cjk_in_strings_but_not_char_literals(self):
        src = (
            'let reason = "头像缓存哈希冲突";\n'
            "let parts = text.split(|c: char| c == ',' || c == '，');\n"
            "const ENDS: [char; 2] = ['。', '\\u{FF0C}'];\n"
            'tracing::warn!("could not save the servers");\n'
        )
        self.assertEqual(lines(src, "rust"), [1])

    def test_test_module_is_ignored(self):
        src = 'fn f() {}\n#[cfg(test)]\nmod tests {\n    const A: &str = "中文";\n}\n'
        self.assertEqual(lines(src, "rust"), [])


if __name__ == "__main__":
    unittest.main()


class TestSourceLangs(unittest.TestCase):
    def test_test_source_sets_are_skipped(self):
        # 测试源集里的假数据和预览名是写死的，不进字符串目录。
        for path in (
            "apps/compose/shared/src/test/kotlin/app/wonton/ui/Foo.kt",
            "apps/compose/androidApp/src/androidTest/kotlin/app/wonton/android/Foo.kt",
            "apps/compose/shared/src/androidHostTest/kotlin/app/wonton/ui/Foo.kt",
        ):
            self.assertIsNone(source_lang(path))

    def test_debug_source_set_is_skipped(self):
        # debug 源集只进 debug 构建（#507：走查 Activity），用户在正式构建里看不到，不算壳用到。
        self.assertIsNone(source_lang("apps/compose/androidApp/src/debug/kotlin/app/wonton/walkthrough/Foo.kt"))

    def test_main_source_sets_are_checked(self):
        for path in (
            "apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/Foo.kt",
            "apps/compose/androidApp/src/main/kotlin/app/wonton/android/Foo.kt",
        ):
            self.assertEqual(source_lang(path), "kotlin")
