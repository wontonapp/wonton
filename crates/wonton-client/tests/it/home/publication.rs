//! 稳定流量不按请求发布首页 ViewModel：只有回声探测的推送按秒算，不跟着请求涨。

use std::time::Instant;

use wonton_client::{Action, DeviceNetwork, HomeStatus};

use super::support::{Home, Phone, WEB_URL, get};

#[tokio::test]
async fn sustained_traffic_does_not_publish_a_home_view_for_each_request() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::NetworkChanged {
                network: DeviceNetwork::Wifi,
            },
        })
        .unwrap();
    let before = phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.proxy.is_some()
                && view.path.is_some()
                && view.details.network == Some(DeviceNetwork::Wifi)
        })
        .await;
    let proxy = before.proxy.unwrap();
    let updates = phone.subscribe_home();
    updates.next().await.unwrap();
    let started = Instant::now();
    let traffic = async {
        for _ in 0..300 {
            let (status, body) = get(&proxy, WEB_URL, "/traffic", true).await;
            assert_eq!((status, body.as_str()), (200, "/traffic"));
            // 同一网络的重复通知要求重算，但状态、路径类型和提示都没有变。
            phone
                .dispatch(Action::Home {
                    action: wonton_protocol::HomeAction::NetworkChanged {
                        network: DeviceNetwork::Wifi,
                    },
                })
                .unwrap();
        }
    };
    tokio::pin!(traffic);
    let mut publications = 0;
    loop {
        tokio::select! {
            () = &mut traffic => break,
            view = updates.next() => {
                let view = view.expect("连接订阅提前结束");
                // 机器忙的时候，网络通知后的确认（4 秒）会超时，连接被换掉：换连接自己会推几条，和这里数的
                // “同一网络没变化也重算”无关，只数仍停在“已连接”的推送。
                publications += u64::from(view.status == HomeStatus::Connected);
            }
        }
    }
    let elapsed = started.elapsed();
    let limit = elapsed.as_secs() + 3;
    eprintln!("300 requests: {publications} home publications in {elapsed:?}, limit {limit}");
    assert!(publications <= limit, "{publications} > {limit}");
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected
                && view.details.sent_packets > before.details.sent_packets
        })
        .await;
}
