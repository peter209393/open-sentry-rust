# Open Sentry Rust

[English](README.md) · **中文**

> 基于 Rust 构建、面向生产运行的自托管 Sentry 兼容错误监控平台。

Open Sentry Rust 支持 Sentry SDK 的事件与 Envelope 接入、Issue 聚合、结构化日志、
Release 与符号文件管理、告警通知、值班升级和可视化 Web 控制台。

**标签：** `rust` · `axum` · `postgresql` · `sentry` · `error-monitoring` ·
`observability` · `self-hosted` · `webhooks` · `prometheus` · `docker`

## 核心能力

- **Sentry 兼容接入**：JSON Store API 与现代 SDK Envelope，支持 `identity`、`gzip`、`deflate/zlib`。
- **Issue 工作流**：聚合、状态流转、评论、合并/拆分、回归检测、手工 fingerprint 和服务端搜索。
- **生产安全能力**：组织隔离、会话认证、RBAC、审计日志、限流、Payload 限制、数据保留与 Prometheus 指标。
- **告警与升级**：邮件、Telegram、Twilio Voice、HMAC 签名 Webhook、值班表、重试、恢复通知和多级升级策略。
- **Release 与符号**：Release 健康、部署、JavaScript Source Map、ELF/Mach-O 符号、PDB 上传校验与事件重新处理。
- **Web 控制台**：项目、服务、日志、事件、Issue、Release、调试文件、告警投递历史和升级策略管理。

## 架构

项目当前为模块化单体：API 与 notification worker 在同一进程中运行，PostgreSQL 同时承担持久化和可靠 outbox。事件、Issue 聚合和告警 outbox 在同一个数据库事务中写入；通知失败会指数退避重试，不会阻塞上报接口。

```text
SDK / curl -> Axum ingest API -> PostgreSQL
                                  |-- events（原始事件）
                                  |-- issues（聚合结果）
                                  |-- releases / debug_files -> Symbol worker
                                  `-- notification_outbox -> Worker -> Email / Telegram / Voice / Webhook

Web UI -> Query API -> PostgreSQL
```

后续可将 worker 拆分，并将接入缓冲迁移至 Kafka/NATS、原始事件迁移至 ClickHouse，无需改变外部 API。

## 快速开始

```bash
cp .env.example .env
docker compose up -d
set -a && source .env && set +a
cargo run
```

首次启动通过以下环境变量创建管理员：

```text
APP__BOOTSTRAP_ADMIN_EMAIL=admin@example.com
APP__BOOTSTRAP_ADMIN_PASSWORD=change-me
```

`change-me` 只适用于本地开发。生产环境应通过密钥管理系统注入独立强密码。密码使用 Argon2 哈希保存，Session Token 仅以 SHA-256 摘要落库。

默认 demo project UUID：`00000000-0000-0000-0000-000000000001`；Sentry 兼容数字 project ID 为 `1`；本地开发 DSN：

```text
http://dev-secret@localhost:8080/1
```

上报事件：

```bash
curl -X POST http://localhost:8080/api/projects/00000000-0000-0000-0000-000000000001/store \
  -H 'content-type: application/json' \
  -H 'x-sentry-auth: dev-secret' \
  -d '{"level":"error","message":"payment timeout","environment":"production","tags":{"service":"checkout"}}'
```

现代 Sentry SDK 可向以下端点发送 Envelope：

```text
POST /api/1/envelope/
X-Sentry-Auth: Sentry sentry_version=7, sentry_key=dev-secret
Content-Type: application/x-sentry-envelope
```

SDK 冒烟测试客户端使用 `sentry 0.48.5`：

```bash
cargo test --bin sentry-smoke -- --test-threads=1
cargo run --bin sentry-smoke -- all
```

## API 概览

| 方法 | 路径 | 用途 |
|---|---|---|
| `GET` | `/health` | 存活与数据库检查 |
| `GET` | `/metrics` | Bearer 保护的 Prometheus 指标 |
| `POST` | `/api/auth/login` | 创建 HttpOnly 控制台会话 |
| `POST` | `/api/{numeric_id}/envelope/` | Sentry Envelope 接入 |
| `POST` | `/api/projects/{id}/store` | 上报 JSON 事件 |
| `GET/PATCH` | `/api/issues/{id}` | 查询或更新 Issue |
| `GET/POST` | `/api/projects/{id}/alert-rules` | 管理告警规则 |
| `GET/POST` | `/api/projects/{id}/releases` | 管理 Release 与健康数据 |
| `GET/POST` | `/api/projects/{id}/debug-files` | 管理 Source Map 与符号文件 |
| `GET/POST` | `/api/projects/{id}/webhooks` | 管理签名 Webhook |
| `GET/POST` | `/api/projects/{id}/on-call-schedules` | 管理值班轮转 |
| `GET/POST` | `/api/projects/{id}/escalation-policies` | 管理多级告警升级 |

完整 API 范围和设计边界请见 [架构说明](docs/architecture.md)。发布门禁、容量阈值、备份与恢复流程见 [生产验收](docs/production-acceptance.md)。

## 生产部署

```bash
cp .env.production.example .env.production
docker compose --env-file .env.production -f docker-compose.production.yml up -d --build
scripts/production-acceptance.sh
```

生产镜像以非 root 用户运行；数据库、后端与前端健康检查控制启动顺序。

## Web 控制台

```bash
cd frontend
cp .env.example .env.local
npm install
npm run dev
```

默认访问 `http://localhost:3000`，并连接 `http://127.0.0.1:8080`。

## 技术栈

- 后端：Rust、Axum、Tokio、SQLx、PostgreSQL
- 前端：Next.js、React、TypeScript
- 运维：Docker Compose、Prometheus 兼容指标

## 说明

本项目使用 **OpenAI ChatGPT 5.6** 开发，AI 开发成本约为 **20 美元（US$20）**。
