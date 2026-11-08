# 每次创建容器时把数据目录里的 config.yaml 修到和注入值一致（Hermes 自己也会改写它，这里只动核心管的两处）。
#
# 1) gateway.trust_env：Matrix 适配器 v0.22.5–v0.22.7 解析代理时不传目标主机，NO_PROXY 不生效；继承
#    HTTPS_PROXY 会把内部的 matrix.wonton.internal 发给出口代理而被 403 拒绝，所以关闭网关的环境代理。
# 2) providers.magpie：模型网关可选（spec：“models”是“有网关就用”）——三个模型变量都有值时保持启用（没有
#    enabled 字段就是启用）；都是空字符串（没接上网关）时标成 disabled，模型设置里不再显示这个不可用的
#    provider，模型要用户自己在设置里配；网关装好后核心重建容器，这里再把它启用回来。用户自己加的 provider
#    不动。enabled 字段是 Hermes 认的（hermes_cli.config_providers.is_provider_enabled）。
# 已经装好的 Bot 也要有这些项，所以不写进包里的 config.yaml。`hermes config set` 在这里用不了：它降权运行，
# 而这时数据目录还属于 root。
import os

from ruamel.yaml import YAML

path = "/opt/data/config.yaml"
yaml = YAML()
with open(path) as f:
    config = yaml.load(f) or {}

changed = False

gateway = config.get("gateway") or {}
if gateway.get("trust_env") is not False:
    gateway["trust_env"] = False
    config["gateway"] = gateway
    changed = True

base = os.environ.get("WONTON_MODEL_BASE_URL", "").strip()
key = os.environ.get("WONTON_MODEL_API_KEY", "").strip()
default = os.environ.get("WONTON_MODEL_DEFAULT", "").strip()
providers = config.get("providers")
magpie = providers.get("magpie") if isinstance(providers, dict) else None
if isinstance(magpie, dict):
    if base and key and default:
        if magpie.pop("enabled", None) is not None:
            changed = True
    elif magpie.get("enabled") is not False:
        magpie["enabled"] = False
        changed = True

if changed:
    with open(path + ".tmp", "w") as f:
        yaml.dump(config, f)
    stat = os.stat(path)
    os.chown(path + ".tmp", stat.st_uid, stat.st_gid)
    os.chmod(path + ".tmp", stat.st_mode)
    os.replace(path + ".tmp", path)
