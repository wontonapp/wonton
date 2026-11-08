//! 这台设备把插件端口映射到回环地址时的状态与 UDP 流向统计。

use serde::{Deserialize, Serialize};

use crate::PortProtocol;

/// 一个插件在这台设备上的端口映射；端口号不改，只监听 `127.0.0.1`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginPortMapping {
    pub plugin_id: String,
    /// 主人打开的手动开关；自动映射不改变它，`true` 也不表示端口已绑定成功。
    pub enabled: bool,
    pub state: PortMappingState,
    /// 清单声明的端口；同号的 TCP 和 UDP 是两项。
    pub ports: Vec<MappedPort>,
}

/// 插件映射的结果类别；文字由原生壳本地化。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PortMappingState {
    Off,
    On,
    PortInUse,
    ServerTooOld,
    TunnelDisconnected,
    /// 服务器上没装这个插件。
    PluginNotInstalled,
    /// 插件的清单没声明这个端口和协议。
    PortNotDeclared,
    /// 插件装了，但没在运行。
    PluginNotRunning,
    /// 插件在运行，服务器却连不上它的服务。
    PluginUnreachable,
    #[serde(other)]
    Other,
}

/// 同号映射的一个本机端口，协议未知时不能尝试绑定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MappedPort {
    pub port: u16,
    pub protocol: PortProtocol,
}

/// 一个 UDP 流向的应用数据包计数；分片拼回后只算一个包，重连后重新计数。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UdpFlowStats {
    /// 只在所属隧道连接内唯一，范围 `0..=16383`。
    pub flow_id: u16,
    pub plugin_id: String,
    pub port: u16,
    /// 本端收到的完整应用包数。
    pub received_packets: u64,
    /// 本端提交给数据报发送队列的完整应用包数，不保证已经送达。
    pub sent_packets: u64,
    /// 超过两片容量而丢弃的应用包数。
    pub oversized_drops: u64,
    /// iroh 只给连接级发送帧数，不能归因到流向；无法测得时为空，不能伪装为零。
    pub buffer_evicted_drops: Option<u64>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Action, PluginsViewModel};

    #[test]
    fn future_mapping_and_transport_categories_do_not_break_the_plugin_view() {
        let mut value = serde_json::to_value(PluginsViewModel::default()).unwrap();
        value["port_mappings"] = json!([{
            "plugin_id": "app.remote", "enabled": true, "state": "future_state",
            "ports": [{"port": 47998, "protocol": "future_transport"}]
        }]);
        let view: PluginsViewModel = serde_json::from_value(value).unwrap();
        assert_eq!(view.port_mappings[0].plugin_id, "app.remote");
        assert_eq!(view.port_mappings[0].state, PortMappingState::Other);
        assert_eq!(view.port_mappings[0].ports[0].protocol, PortProtocol::Other);
    }

    #[test]
    fn mapping_action_keeps_plugin_and_requested_switch() {
        let value = json!({"type": "set_port_mapping", "plugin_id": "app.remote", "enabled": true});
        let action: Action = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            action,
            Action::SetPortMapping {
                plugin_id: "app.remote".into(),
                enabled: true
            }
        );
        assert_eq!(serde_json::to_value(action).unwrap(), value);
    }

    #[test]
    fn flow_statistics_distinguish_unknown_evictions_from_zero() {
        let value = json!({
            "flow_id": 16383, "plugin_id": "app.remote", "port": 47998,
            "received_packets": 6, "sent_packets": 8, "oversized_drops": 2,
            "buffer_evicted_drops": null
        });
        let stats: UdpFlowStats = serde_json::from_value(value).unwrap();
        assert_eq!(stats.plugin_id, "app.remote");
        assert_eq!(stats.buffer_evicted_drops, None);
        let measured = UdpFlowStats {
            buffer_evicted_drops: Some(0),
            ..stats.clone()
        };
        let decoded: UdpFlowStats =
            serde_json::from_value(serde_json::to_value(&measured).unwrap()).unwrap();
        assert_eq!(decoded, measured);
        assert_ne!(decoded, stats);
    }
}
