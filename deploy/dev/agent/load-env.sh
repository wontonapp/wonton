# shellcheck shell=bash
# dev.sh 和 smoke.sh 共用：读配置文件（KEY=VALUE，参考 .env.example）。
#
# 环境里已经设置的变量优先于文件里的值，和 docker compose 读 .env 的规则一致：命令行上的
#   WONTON_AGENT_PROJECT=wonton-dev-agent-x ./dev.sh up
# 不会被 .env 里的同名行盖掉。否则 .env 里写着默认项目名时，换了项目名的 reset 会作用到默认的那一套。

load_env_file() {
  local file="$1" name saved=""
  [[ -f "$file" ]] || return 0
  while IFS= read -r name; do
    [[ -z "${!name+x}" ]] || saved+="$(printf '%s=%q;' "$name" "${!name}")"
  done < <(sed -n 's/^[[:space:]]*\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' "$file")
  set -a
  # shellcheck disable=SC1090
  source "$file"
  set +a
  eval "$saved"
}
