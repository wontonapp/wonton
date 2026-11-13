use serde_json::{Value, json};
use wonton_client::{Action, Rgb, TabBridgeAction, TabPlatform, TabRef, accent_hex};

use super::driver::Driver;
use super::support::{environment, open_page};
use crate::plugins::rig::Rig;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要 Docker，官方 node:22 运行 MCP Apps SDK 2.0.4 的真实桥握手"]
async fn official_sdk_roundtrip() {
    for platform in [TabPlatform::Ios, TabPlatform::Android] {
        let (rig, tab) = open_page(true).await;
        let mut environment = environment();
        environment.platform = platform;
        let mut driver = Driver::start(&rig, &tab, environment);
        let ready = driver.receive();
        assert_eq!(ready["ready"], true);
        assert!(driver.initialized);
        assert_eq!(ready["context"]["theme"], "light");
        assert_eq!(
            ready["capabilities"],
            json!({"openLinks":{},"message":{"text":{}}})
        );
        let light = driver.command("inspect", Value::Null);
        // 铺满时页面自己的背景要露到状态栏下面，桥不能在 <html> 上盖一层底色（ADR-0095）。
        assert!(light["css"]["background-color"].is_null());
        assert_eq!(light["css"]["color-scheme"], "light");
        assert_eq!(light["css"]["--wonton-inset-top"], "88px");
        assert_eq!(
            light["css"]["--border-radius-md"],
            if platform == TabPlatform::Ios {
                "14px"
            } else {
                "12px"
            }
        );
        let fullscreen = driver.command("requestDisplayMode", json!({"mode":"fullscreen"}));
        assert_eq!(fullscreen["result"]["mode"], "fullscreen");
        rig.app
            .dispatch(Action::SetAccentColor {
                color: Rgb {
                    red: 30,
                    green: 120,
                    blue: 210,
                },
            })
            .unwrap();
        driver.environment.dark = true;
        driver.environment.inset_bottom = 34.0;
        driver.environment.safe_area.left = 59.0;
        driver.update_context();
        let dark = driver.command("inspect", Value::Null);
        assert_eq!(dark["context"]["theme"], "dark");
        assert_eq!(dark["context"]["displayMode"], "fullscreen");
        assert_eq!(dark["contextChanges"][0], dark["context"]);
        assert!(dark["css"]["background-color"].is_null());
        assert_eq!(
            dark["css"]["--wonton-accent-fill"],
            accent_hex(rig.app.appearance().dark.accent_fill)
        );
        assert_eq!(dark["css"]["--wonton-inset-bottom"], "34px");
        assert_eq!(dark["css"]["--wonton-inset-left"], "59px");
        assert_eq!(dark["css"]["--wonton-inset-right"], "0px");
        assert_eq!(dark["css"]["color-scheme"], "dark");
        assert!(
            dark["context"]["styles"]["variables"]
                .get("--wonton-accent-fill")
                .is_none()
        );
        // 应用一律整页打开（ADR-0095）：要 inline 也照样铺满，宿主如实回报 fullscreen。
        let inline = driver.command("requestDisplayMode", json!({"mode":"inline"}));
        assert_eq!(inline["result"]["mode"], "fullscreen");
        driver.environment.dark = false;
        driver.update_context();
        assert_eq!(
            driver.command("inspect", Value::Null)["context"]["theme"],
            "light"
        );
        assert_page_actions(&mut driver, &rig, &tab);
        driver.finish();
    }
}

/// 页面发起的动作交给壳，壳发来的事件页面收得到。
fn assert_page_actions(driver: &mut Driver, rig: &Rig, tab: &TabRef) {
    driver.command("openLink", json!({"url":"https://example.org/"}));
    driver.command(
        "sendMessage",
        json!({"role":"user","content":[{"type":"text","text":"讨论文章"}]}),
    );
    driver.command("sendSizeChanged", json!({"height":500}));
    assert_eq!(
        driver.actions,
        vec![
            TabBridgeAction::OpenLink {
                url: "https://example.org/".to_owned()
            },
            TabBridgeAction::DraftMessage {
                text: "讨论文章".to_owned()
            },
        ]
    );
    driver.send(json!({"script":rig.app.tab_reselected_script()}));
    driver.send(json!({"script":rig.app.tab_connection_script(tab.clone())}));
    let events = driver.command("inspect", Value::Null);
    assert_eq!(
        events["events"],
        json!([
            {"type":"wonton:tab-reselected","detail":null},
            {"type":"wonton:connection","detail":{"connected":true}},
        ])
    );
}
