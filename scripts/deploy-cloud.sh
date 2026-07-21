#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

ENV_FILE=".env.cloud"
COMPOSE_FILE="docker-compose.cloud.yml"

echo "========================================"
echo "  Open Sentry 云服务器部署脚本"
echo "========================================"

# 检查必要命令
for cmd in docker docker compose curl; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "错误: 未找到 '$cmd'，请先安装 Docker + Docker Compose + curl"
        exit 1
    fi
done

# 检查环境文件
if [[ ! -f "$ENV_FILE" ]]; then
    echo "错误: 找不到 $ENV_FILE"
    echo "请先复制并编辑环境变量文件:"
    echo "  cp .env.cloud.example .env.cloud   # 或手动创建"
    exit 1
fi

# 检查 Caddyfile 域名是否已修改
if grep -q 'sentry.example.com' Caddyfile; then
    echo "警告: Caddyfile 中仍使用 sentry.example.com，请替换为你的真实域名"
    read -r -p "是否继续? [y/N] " confirm
    if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
        exit 1
    fi
fi

# 加载环境变量
set -a
source "$ENV_FILE"
set +a

echo ""
echo "[1/4] 拉取/构建镜像..."
docker compose -f "$COMPOSE_FILE" pull 2>/dev/null || true
docker compose -f "$COMPOSE_FILE" build --no-cache

echo ""
echo "[2/4] 启动数据库与后端..."
docker compose -f "$COMPOSE_FILE" up -d postgres backend

# 等待后端健康
for i in {1..30}; do
    if docker compose -f "$COMPOSE_FILE" ps backend | grep -q healthy; then
        echo "后端服务已就绪"
        break
    fi
    echo "等待后端健康检查... ($i/30)"
    sleep 2
done

if ! docker compose -f "$COMPOSE_FILE" ps backend | grep -q healthy; then
    echo "错误: 后端服务未通过健康检查"
    docker compose -f "$COMPOSE_FILE" logs --tail=50 backend
    exit 1
fi

echo ""
echo "[3/4] 启动前端与 Caddy..."
docker compose -f "$COMPOSE_FILE" up -d frontend caddy

echo ""
echo "[4/4] 等待 Caddy 启动..."
sleep 5

echo ""
echo "========================================"
echo "  部署完成"
echo "========================================"
echo ""
echo "  Web 控制台: ${PUBLIC_BASE_URL}"
echo "  管理员邮箱: ${ADMIN_EMAIL}"
echo "  健康检查: ${PUBLIC_BASE_URL}/health"
echo "  监控指标: ${PUBLIC_BASE_URL}/metrics"
echo ""
echo "  DSN 示例: https://${INGEST_API_KEY}@${PUBLIC_BASE_URL#https://}/1"
echo ""
echo "常用命令:"
echo "  查看日志:   docker compose -f $COMPOSE_FILE logs -f"
echo "  查看状态:   docker compose -f $COMPOSE_FILE ps"
echo "  重启服务:   docker compose -f $COMPOSE_FILE restart"
echo "  备份数据库: ./scripts/backup-postgres.sh"
echo ""
