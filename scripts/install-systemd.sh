#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
   echo "错误: 请使用 sudo 运行此脚本"
   exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="/opt/open-sentry"

echo "========================================"
echo "  Open Sentry Systemd 服务安装"
echo "========================================"

# 创建运行用户
if ! id -u sentry &>/dev/null; then
    echo "创建 sentry 用户..."
    useradd --system --create-home --home-dir /opt/sentry sentry
fi

# 复制项目文件（如果当前目录不是 /opt/open-sentry）
if [[ "$(cd "$SCRIPT_DIR/.." && pwd)" != "$INSTALL_DIR" ]]; then
    echo "复制项目到 $INSTALL_DIR ..."
    mkdir -p "$INSTALL_DIR"
    cp -r "$SCRIPT_DIR/.."/* "$INSTALL_DIR/"
fi

chown -R sentry:sentry "$INSTALL_DIR"

# 编译后端
echo "编译 Rust 后端..."
cd "$INSTALL_DIR"
sudo -u sentry bash -c 'cargo build --release'

# 构建前端
echo "构建 Next.js 前端..."
cd "$INSTALL_DIR/frontend"
sudo -u sentry bash -c 'npm run build'

# 创建日志目录
mkdir -p /var/log/open-sentry
chown sentry:sentry /var/log/open-sentry

# 安装 systemd 服务
cp "$SCRIPT_DIR"/open-sentry-*.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable open-sentry-backend
systemctl enable open-sentry-frontend

# 先启动基础设施
echo "启动 Docker 基础设施 (PostgreSQL + Caddy)..."
cd "$INSTALL_DIR"
docker compose -f docker-compose.infra.yml up -d

# 等待 PG 就绪
for i in {1..30}; do
    if docker compose -f docker-compose.infra.yml ps postgres | grep -q healthy; then
        echo "PostgreSQL 已就绪"
        break
    fi
    echo "等待 PostgreSQL... ($i/30)"
    sleep 2
done

# 启动主程序
echo "启动主程序服务..."
systemctl start open-sentry-backend
sleep 3
systemctl start open-sentry-frontend

echo ""
echo "========================================"
echo "  Systemd 服务安装完成"
echo "========================================"
echo ""
echo "  查看状态:"
echo "    systemctl status open-sentry-backend"
echo "    systemctl status open-sentry-frontend"
echo "    systemctl status docker-compose.infra"
echo ""
echo "  查看日志:"
echo "    journalctl -u open-sentry-backend -f"
echo "    journalctl -u open-sentry-frontend -f"
echo ""
echo "  管理命令:"
echo "    重启后端: systemctl restart open-sentry-backend"
echo "    重启前端: systemctl restart open-sentry-frontend"
echo "    停止全部: systemctl stop open-sentry-backend open-sentry-frontend"
echo ""
