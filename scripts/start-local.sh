#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

ENV_FILE=".env.local"
COMPOSE_FILE="docker-compose.infra.yml"

echo "========================================"
echo "  Open Sentry 本地主程序 + Docker 基础设施"
echo "========================================"

# 检查必要命令
for cmd in docker docker compose cargo npm; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "错误: 未找到 '$cmd'，请先安装"
        exit 1
    fi
done

# 检查环境文件
if [[ ! -f "$ENV_FILE" ]]; then
    echo "错误: 找不到 $ENV_FILE"
    echo "请先复制并编辑: cp .env.local.example .env.local"
    exit 1
fi

# 检查域名是否已修改
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
echo "[1/4] 启动 Docker 基础设施 (PostgreSQL + Caddy)..."
docker compose -f "$COMPOSE_FILE" up -d

# 等待 PostgreSQL 就绪
for i in {1..30}; do
    if docker compose -f "$COMPOSE_FILE" ps postgres | grep -q healthy; then
        echo "PostgreSQL 已就绪"
        break
    fi
    echo "等待 PostgreSQL... ($i/30)"
    sleep 2
done

if ! docker compose -f "$COMPOSE_FILE" ps postgres | grep -q healthy; then
    echo "错误: PostgreSQL 未通过健康检查"
    docker compose -f "$COMPOSE_FILE" logs --tail=50 postgres
    exit 1
fi

echo ""
echo "[2/4] 编译并启动 Rust 后端..."
echo "（首次编译可能需要 5-15 分钟，请耐心等待）"
cargo build --release

# 确保日志目录存在
mkdir -p /var/log/open-sentry 2>/dev/null || true

nohup ./target/release/open-sentry > /var/log/open-sentry/backend.log 2>&1 &
echo $! > /tmp/open-sentry-backend.pid
echo "后端 PID: $(cat /tmp/open-sentry-backend.pid)"

# 等待后端启动
for i in {1..15}; do
    if curl -sf http://127.0.0.1:8080/health > /dev/null 2>&1; then
        echo "后端健康检查通过"
        break
    fi
    echo "等待后端就绪... ($i/15)"
    sleep 2
done

echo ""
echo "[3/4] 构建并启动 Next.js 前端..."
cd frontend
npm run build
cd ..

nohup bash -c 'cd frontend && npm run start' > /var/log/open-sentry/frontend.log 2>&1 &
echo $! > /tmp/open-sentry-frontend.pid
echo "前端 PID: $(cat /tmp/open-sentry-frontend.pid)"

# 等待前端启动
for i in {1..15}; do
    if curl -sf http://127.0.0.1:3000/ > /dev/null 2>&1; then
        echo "前端健康检查通过"
        break
    fi
    echo "等待前端就绪... ($i/15)"
    sleep 2
done

echo ""
echo "[4/4] 部署状态检查..."
if docker compose -f "$COMPOSE_FILE" ps | grep -q "caddy"; then
    echo "Caddy 运行中 (host 网络模式，监听宿主机 80/443)"
fi

echo ""
echo "========================================"
echo "  本地主程序部署完成"
echo "========================================"
echo ""
echo "  Web 控制台: ${APP__PUBLIC_BASE_URL}"
echo "  管理员邮箱: ${APP__BOOTSTRAP_ADMIN_EMAIL}"
echo "  本地后端:   http://127.0.0.1:8080"
echo "  本地前端:   http://127.0.0.1:3000"
echo ""
echo "  日志文件:"
echo "    后端: /var/log/open-sentry/backend.log"
echo "    前端: /var/log/open-sentry/frontend.log"
echo "    Caddy: docker compose -f $COMPOSE_FILE logs caddy"
echo ""
echo "  停止命令:"
echo "    kill \$(cat /tmp/open-sentry-backend.pid) \$(cat /tmp/open-sentry-frontend.pid)"
echo "    docker compose -f $COMPOSE_FILE down"
echo ""
