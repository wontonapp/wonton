# dev.sh install、upgrade 共用的打包和预览。dev.sh 在用到这些子命令时才 source 本文件：
# dev_test.py 会把 dev.sh 单独复制到假检出树里跑，本文件不在旁边也不能碍事。

# 插件包：compose 文件原样发送；目录打成 tar 归档发送（compose 引用了包里的文件时，如 Hermes 的 config.yaml）。
package() {
  if [[ -d "$1" ]]; then
    COPYFILE_DISABLE=1 tar -C "$1" -cf - .
  else
    cat "$1"
  fi
}

package_type() {
  if [[ -d "$1" ]]; then echo application/x-tar; else echo application/yaml; fi
}

# 预览一个插件包（只读），输出 InstallPreview；接口出错时输出错误正文。
preview_package() {
  package "$1" | api POST /_wonton/api/plugins/preview -H "Content-Type: $(package_type "$1")" --data-binary @-
}

# 把预览讲给人看，并取出要授予的权限（全局 GRANTS，逗号分隔）和插件 ID（全局 PLUGIN_ID）。
# 两个变量跨函数用，调用方接着拿去发请求；装不了（blocked）时退出。
preview_read() {
  local preview="$1" blocked
  # 装不了（架构不对等）时预览里可以没有 id、标题这些：先讲 blocked，缺字段不该先死在取字段上。
  blocked="$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("blocked") or "")' <<<"$preview" 2>/dev/null || true)"
  [[ -z "$blocked" ]] || die "这台主机上装不了：$blocked"
  GRANTS="$(json_field permissions <<<"$preview" 2>/dev/null | python3 -c 'import json,sys
print(",".join(p["permission"] for p in json.load(sys.stdin)))' 2>/dev/null || true)"
  PLUGIN_ID="$(json_field id <<<"$preview" 2>/dev/null || true)"
  [[ -n "$PLUGIN_ID" ]] || die "预览里没有插件 ID：$preview"
  echo "插件：$PLUGIN_ID（$(json_field title <<<"$preview" 2>/dev/null || echo ？)）" >&2
  echo "需要的权限：${GRANTS:-无}" >&2
  python3 -c 'import json,sys
for notice in json.load(sys.stdin).get("notices") or []:
    print("提示：" + notice["text"])' <<<"$preview" >&2
}

# 密钥放头里，查询串会进日志：把 Wonton-Plugin-Secrets 追加进 $1 命名的头数组，值为空时不加。
# 引用名不能和调用方的数组同名（headers=headers 是自引用，bash 报 circular）。
secret_header() {
  local -n hdrs=$1
  [[ -n "${2:-}" ]] || return 0
  hdrs+=(-H "Wonton-Plugin-Secrets: $(python3 -c 'import base64,json,sys;print(base64.urlsafe_b64encode(json.dumps(json.loads(sys.argv[1])).encode()).decode().rstrip("="))' "$2")")
}

# 插件装着没有：安装接口对已装的插件报错退出，复跑时改走升级（换容器，不吊销 Bot 的设备）。
plugin_installed() {
  api GET /_wonton/api/plugins | python3 -c 'import json, sys
sys.exit(0 if any(p["id"] == sys.argv[1] for p in json.load(sys.stdin)["plugins"]) else 3)' "$1"
}

install() {
  local file="$1" yes="${2:-}" secrets="${3:-}" type preview answer out
  local -a headers statuses
  [[ -e "$file" ]] || die "找不到插件包：$file"
  type="$(package_type "$file")"
  preview="$(preview_package "$file")" || die "预览失败：$preview"
  # 已装过的插件，安装预览就是 blocked（already installed）：先取 ID，装了就走升级。预览里没有 id（无效包）
  # 就留给下面的 preview_read 打印原因，别先死在取字段上。
  PLUGIN_ID="$(json_field id <<<"$preview" 2>/dev/null || true)"
  if [[ -n "$PLUGIN_ID" ]] && plugin_installed "$PLUGIN_ID"; then
    echo "$PLUGIN_ID 已经装过，改走升级" >&2
    out="$(upgrade "$file" "$yes" "$secrets" 2>&1)" || {
      if grep -q 'nothing to update' <<<"$out"; then
        echo "$PLUGIN_ID 已经是这个版本，不用动" >&2
        return 0
      fi
      printf '%s\n' "$out" >&2
      return 1
    }
    printf '%s\n' "$out"
    return 0
  fi
  preview_read "$preview"
  if [[ "$yes" != "--yes" ]]; then
    read -r -p "确认安装？[y/N] " answer
    [[ "$answer" == y || "$answer" == Y ]] || die "已取消"
  fi
  headers=(-H "Content-Type: $type")
  secret_header headers "$secrets"
  package "$file" | api POST "/_wonton/api/plugins?grant=$GRANTS" "${headers[@]}" --data-binary @- || {
    statuses=("${PIPESTATUS[@]}")
    if [[ "${statuses[1]}" -eq 28 ]]; then
      printf '核心可能仍在安装，用 %q status 查\n' "$here/dev.sh" >&2
    fi
    # pipefail 返回最右侧的非零状态；打包失败不能被成功的 API 掩盖。
    if [[ "${statuses[1]}" -ne 0 ]]; then return "${statuses[1]}"; fi
    return "${statuses[0]}"
  }
  echo
}

# 升级已装的插件：走核心的升级接口，换容器不吊销 Bot 的 Matrix 设备，数据和密钥都留着
# （uninstall 再 install 才会吊销设备，Bot 读不到旧房间的加密历史）。
upgrade() {
  local file="$1" yes="${2:-}" secrets="${3:-}" type id preview answer
  local -a headers
  [[ -e "$file" ]] || die "找不到插件包：$file"
  type="$(package_type "$file")"
  # 升级预览按插件 ID 寻址，先用安装预览（只读）读出包里的 ID。
  preview="$(preview_package "$file")" || die "预览失败：$preview"
  id="$(json_field id <<<"$preview" 2>/dev/null)" || die "预览失败：$preview"
  preview="$(package "$file" | api POST "/_wonton/api/plugins/$id/upgrade/preview" -H "Content-Type: $type" --data-binary @-)" ||
    die "预览失败：$preview"
  preview_read "$preview"
  if [[ "$yes" != "--yes" ]]; then
    read -r -p "确认升级？[y/N] " answer
    [[ "$answer" == y || "$answer" == Y ]] || die "已取消"
  fi
  headers=(-H "Content-Type: $type")
  secret_header headers "$secrets"
  package "$file" | api POST "/_wonton/api/plugins/$id/upgrade?grant=$GRANTS" "${headers[@]}" --data-binary @-
  echo
}
