"""两个壳的对齐检查：给一个小仓库（Rust 导出 + Swift 壳 + Kotlin 壳），断言哪些项报成只有一边用到。

运行：python3 -m unittest discover -s scripts/shell_parity -t scripts
"""

from __future__ import annotations

import os
import tempfile
import unittest
from typing import Dict

from shell_parity.items import collect_items, usage
from shell_parity.rust import read_exports
from shell_parity.shells import lower_camel, read_shells

RUST = """
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Action {
    PairHome { code: String },
    RefreshHome,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HomeViewModel {
    pub device_name: String,
    pub web_url: Option<String>,
    pub status: HomeStatus,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[expect(
    clippy::large_enum_variant,
    reason = "跨行的属性"
)]
pub enum HomeStatus {
    Connecting,
    Connected { latency_ms: u32 },
    Failed { failure: HomeFailure },
    #[serde(other)]
    Unknown,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HomeFailure {
    pub detail: String,
}

#[derive(uniffi::Object)]
pub struct ClientCore {}

#[uniffi::export]
impl ClientCore {
    /// 文档注释
    pub fn home(&self) -> HomeViewModel {
        todo!()
    }
}
"""

SWIFT = """
import WontonClient
struct HomeView: View {
    let model: HomeViewModel
    var body: some View {
        Text(model.deviceName)          // model.webUrl 只在注释里
        switch model.status {
        case .connecting: Text("…")
        case let .connected(latency): Text("\\(latency)")
        case .failed: EmptyView()
        case .unknown: EmptyView()
        }
        Button("") { core.dispatch(action: .pairHome(code: "")) }
        Button("") { core.dispatch(action: .refreshHome) }
    }
}
"""

KOTLIN = """
package app.wonton.ui
import app.wonton.protocol.HomeViewModel
import androidx.compose.web.webUrl
@Composable
fun HomeScreen(viewModel: HomeViewModel) {
    Text(viewModel.deviceName)
    when (val status = viewModel.status) {
        is HomeStatus.Connected -> Text("${status.latencyMs}")
        else -> Unit
    }
    Button(onClick = { core.dispatch(Action.PairHome("")) }) {}
}
"""


def write(root: str, files: Dict[str, str]) -> None:
    for path, text in files.items():
        full = os.path.join(root, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as handle:
            handle.write(text)


class ParityTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        write(
            self.tmp.name,
            {
                "crates/wonton-protocol/src/lib.rs": RUST,
                "apps/ios/Wonton/HomeView.swift": SWIFT,
                "apps/ios/WontonTests/HomeTests.swift": "let x = vm.webUrl",
                "apps/compose/shared/src/androidMain/HomeScreen.kt": KOTLIN,
            },
        )

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def where(self) -> Dict[str, set]:
        exports = read_exports(self.tmp.name, ["crates/wonton-protocol/src"])
        self.assertEqual(exports.unparsed, [])
        items = collect_items(exports, set())
        return usage(exports, set(), items, read_shells(self.tmp.name)).shells

    def test_one_shell_items(self) -> None:
        used = self.where()
        self.assertEqual(used["HomeViewModel.device_name"], {"iOS", "Android"})
        self.assertEqual(used["Action.PairHome"], {"iOS", "Android"})
        self.assertEqual(used["Action.RefreshHome"], {"iOS"})
        self.assertEqual(used["HomeStatus.Connected"], {"iOS", "Android"})
        # Kotlin 把不带数据的分支并进 else，Swift 逐个写出：整个枚举算一项，Kotlin 提到枚举名就算用到。
        self.assertEqual(used["HomeStatus"], {"iOS", "Android"})
        self.assertEqual(used["HomeStatus.Failed"], {"iOS"})

    def test_comments_tests_and_imports_do_not_count(self) -> None:
        self.assertEqual(self.where()["HomeViewModel.web_url"], set())

    def test_catch_all_and_error_types_are_not_items(self) -> None:
        used = self.where()
        self.assertNotIn("HomeStatus.Unknown", used)
        self.assertNotIn("HomeFailure.detail", used)

    def test_unparsed_derive_is_reported(self) -> None:
        write(self.tmp.name, {"crates/wonton-protocol/src/odd.rs": "#[derive(uniffi::Record)]\nstruct Hidden {}\n"})
        exports = read_exports(self.tmp.name, ["crates/wonton-protocol/src"])
        self.assertEqual(exports.unparsed, ["crates/wonton-protocol/src/odd.rs:1"])

    def test_lower_camel_matches_uniffi(self) -> None:
        self.assertEqual(lower_camel("udp_buffer_evicted_datagrams"), "udpBufferEvictedDatagrams")
        self.assertEqual(lower_camel("PairHome"), "pairHome")
        self.assertEqual(lower_camel("ip_v4_address"), "ipV4Address")


if __name__ == "__main__":
    unittest.main()
