# 配了代理时，Hermes 的 Matrix 适配器（v0.22.5 实测）读了 HTTPS_PROXY 就不看 NO_PROXY，连内部的
# matrix.wonton.internal 也发给出口容器，被 403 拒绝，所以关掉网关读代理变量的开关。
# 已经装好的 Bot 也要有这一项，所以不写进 config.yaml。`hermes config set` 在这里用不了：它降权运行，
# 而这时数据目录还属于 root。
import os

from ruamel.yaml import YAML

path = "/opt/data/config.yaml"
yaml = YAML()
with open(path) as f:
    config = yaml.load(f) or {}
gateway = config.get("gateway") or {}
if gateway.get("trust_env") is not False:
    gateway["trust_env"] = False
    config["gateway"] = gateway
    with open(path + ".tmp", "w") as f:
        yaml.dump(config, f)
    stat = os.stat(path)
    os.chown(path + ".tmp", stat.st_uid, stat.st_gid)
    os.chmod(path + ".tmp", stat.st_mode)
    os.replace(path + ".tmp", path)
