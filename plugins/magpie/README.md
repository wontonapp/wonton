# magpie

## 服务商命令验证

`test-providers.sh` 需要 Docker，使用清单锁定的 magpie 镜像与仅存在于测试中的 OpenAI 兼容桩。
测试容器只接入内部网络，不使用真实厂商或真实 key；退出时删除自己创建的容器、网络、卷和镜像标签。

```sh
~/wonton-dev/stack-slot.sh acquire v2-core
WONTON_STACK_SLOT_HELD=v2-core nice -n 19 sh plugins/magpie/test-providers.sh
~/wonton-dev/stack-slot.sh release v2-core
```

覆盖只测试不保存、自定义保存、拒绝 key、断连和超时，以及从另一容器用插件网关凭据请求
`group/wonton-default`，验证请求实际使用所填的服务商和模型。默认组被主人修改后不覆盖，规则见
[默认模型设计](../../docs/adr/0008-model-gateway-magpie.md#补充默认模型2026-10-30)。

## 图标

`icon.png` 来自与 `ghcr.io/yetone/magpie` 对应的上游仓库 [yetone/magpie 的 `build/icon/magpie.svg`](https://github.com/yetone/magpie/blob/50265256ba56ac164a680da0689e1c6d90dac471/build/icon/magpie.svg)，
固定提交为 `50265256ba56ac164a680da0689e1c6d90dac471`，按 [MIT 许可证](LICENSE.icon)再分发。
原始文件保存在 [icon.svg](icon.svg)，字节未改；PNG 仅等比缩放、加纯白底并栅格化为 512×512，不改色、不变形。

Magpie 名称和标志是上游项目及其权利人 yetone 的商标。这里只用于识别上游应用，不表示 Wonton 是其官方出品，
也不暗示合作或背书；MIT 不授予商标权。原始 PNG 文本元数据另附来源、版权和许可声明。
