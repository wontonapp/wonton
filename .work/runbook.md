# 栈起来后的步骤
export WONTON_DEV_PROJECT=wonton-tt-linksv WONTON_DEV_PORT=19680 WONTON_DEV_TUNNEL_PORT=19682
BASE=192.168.0.69.nip.io
CORE=$(docker ps -q -f label=com.docker.compose.project=wonton-tt-linksv -f label=com.docker.compose.service=core)
docker exec "$CORE" wonton-core pair 192.168.0.69:19682        # 两个设备各一次
dev.sh install plugins/examples/whoami --yes
dev.sh install plugins/examples/echo-bot --yes
注册 family；配对两机；跳过引导；发起私聊 @family:<BASE>；family 接邀请；按 order.py 依次发消息
