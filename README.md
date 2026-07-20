# Open Sentry Rust

一个使用 Rust 构建、面向生产运行的 Sentry 兼容错误监控平台：支持 SDK/Envelope
接入、Issue 聚合、结构化日志与事件查询、项目和服务视图、告警通知、审计、RBAC
以及可视化 Web 控制台。

## 架构

当前采用模块化单体，API 和 notification worker 运行在同一进程，PostgreSQL 同时承担持久化与可靠 outbox。这样可以先用一套二进制跑通闭环；流量增长后可独立运行 worker，并将原始事件迁移到 ClickHouse、将接入缓冲迁移到 Kafka/NATS，而无需改变外部 API。

```text
SDK / curl -> Axum ingest API -> PostgreSQL
                                  |-- events (原始事件)
                                  |-- issues (聚合结果)
                                  |-- releases / debug_files -> Symbol worker
                                  `-- notification_outbox -> Worker -> Email / Telegram / Voice / Webhook

Web UI -> Query API -> PostgreSQL
```

核心一致性：事件、Issue 聚合和告警 outbox 在同一个数据库事务内写入；通知发送失败会指数退避重试，不会阻塞上报接口。

## 快速开始

```bash
cp .env.example .env
docker compose up -d
set -a && source .env && set +a
cargo run
```

Web 控制台使用数据库会话认证。首次启动会根据以下环境变量创建管理员：

```text
APP__BOOTSTRAP_ADMIN_EMAIL=admin@example.com
APP__BOOTSTRAP_ADMIN_PASSWORD=change-me
```

`change-me` 仅用于本地开发；生产部署必须通过密钥管理系统注入独立强密码。
密码以 Argon2 哈希保存，会话令牌仅以 SHA-256 摘要落库，管理 API 同时执行组织/项目隔离检查。

默认 demo project id：`00000000-0000-0000-0000-000000000001`。
对外兼容 Sentry 的数字 project id 为 `1`，开发 DSN 为
`http://dev-secret@localhost:8080/1`。

上报事件：

```bash
curl -X POST http://localhost:8080/api/projects/00000000-0000-0000-0000-000000000001/store \
  -H 'content-type: application/json' \
  -H 'x-sentry-auth: dev-secret' \
  -d '{"level":"error","message":"payment timeout","environment":"production","tags":{"service":"checkout"}}'
```

现代 Sentry SDK 可将 Envelope 发送至：

```text
POST /api/1/envelope/
X-Sentry-Auth: Sentry sentry_version=7, sentry_key=dev-secret
Content-Type: application/x-sentry-envelope
```

Envelope 支持 `identity`、`gzip` 和 `deflate/zlib` 编码。压缩请求体、解压后
Payload、Item 数量和单 Event 分别限制为 5 MiB、20 MiB、100 和 1 MiB。
Event 与 Transaction 会进入结构化事件表；Attachment、Logs、Session 等其他
SDK Item 会原样写入 `envelope_items`，避免未知或新协议数据静默丢失。

完整 SDK 冒烟测试客户端使用 `sentry 0.48.5`：

```bash
cargo test --bin sentry-smoke -- --test-threads=1
cargo run --bin sentry-smoke -- all
```

默认 DSN 是 `http://dev-secret@127.0.0.1:8080/1`，也可通过 `SENTRY_DSN`
覆盖。`all` 会发送 message、标准 Error、自定义 Event、Attachment、Transaction、
结构化 Logs 和一个被捕获的 panic；也可以用对应名称单独执行某一类。

查询 Issue：

```bash
curl 'http://localhost:8080/api/projects/00000000-0000-0000-0000-000000000001/issues?status=unresolved'
```

创建 Telegram 告警规则（`target` 是 chat id）：

```bash
curl -X POST http://localhost:8080/api/projects/00000000-0000-0000-0000-000000000001/alert-rules \
  -H 'content-type: application/json' \
  -d '{"name":"production errors","level":"error","environment":"production","cooldown_seconds":300,"channel":"telegram","target":"123456789"}'
```

创建电话叫醒规则前配置 Twilio Voice（主叫号码必须具备 Voice 能力）：

```bash
export APP__TWILIO__ACCOUNT_SID='AC...'
export APP__TWILIO__AUTH_TOKEN='...'
export APP__TWILIO__FROM_NUMBER='+1...'

curl -X POST http://localhost:8080/api/projects/00000000-0000-0000-0000-000000000001/alert-rules \
  -H 'content-type: application/json' \
  -d '{"name":"night wake-up","level":"error","environment":"production","cooldown_seconds":300,"channel":"voice_call","target":"+60123456789"}'
```

`target` 必须使用 E.164 格式。马来西亚号码应写成 `+60` 加去掉开头 `0` 的本地号码，
例如本地 `012-3456789` 写作 `+60123456789`。请勿把真实号码或 Twilio 密钥提交到仓库。

## API v0

| 方法 | 路径 | 用途 |
|---|---|---|
| `GET` | `/health` | 存活与数据库检查 |
| `GET` | `/metrics` | Bearer 保护的 Prometheus 指标 |
| `POST` | `/api/auth/login` | 创建 HttpOnly 控制台会话 |
| `POST` | `/api/auth/logout` | 注销当前会话 |
| `GET` | `/api/auth/me` | 查询当前用户 |
| `GET` | `/api/audit-logs` | 当前组织的安全审计日志 |
| `POST` | `/api/projects/{id}/store` | 上报 JSON 事件 |
| `POST` | `/api/{numeric_id}/envelope/` | Sentry Envelope 接入 |
| `GET` | `/api/projects/{id}/issues` | Issue 列表 |
| `GET/PATCH` | `/api/issues/{id}` | Issue 详情/状态变更 |
| `GET` | `/api/issues/{id}/events` | Issue 下的事件 |
| `GET/POST` | `/api/projects/{id}/alert-rules` | 查询/创建告警规则 |
| `GET/POST` | `/api/projects/{id}/releases` | Release 健康与版本管理 |
| `GET/POST` | `/api/projects/{id}/debug-files` | Source Map / 原生符号管理 |
| `GET/POST` | `/api/projects/{id}/webhooks` | HMAC 签名 Webhook 管理 |
| `GET/POST` | `/api/projects/{id}/on-call-schedules` | 值班轮转管理 |
| `GET/POST` | `/api/projects/{id}/escalation-policies` | 多级告警升级策略 |

## 生产能力与演进路线

当前接入层默认按 DSN/API 凭证执行每分钟 600 次严格限流，超限返回 `429` 和
`Retry-After: 60`。可通过 `APP__INGEST_RATE_LIMIT_PER_MINUTE` 调整。运行指标通过
`Authorization: Bearer $APP__METRICS_API_KEY` 读取 `/metrics`。Event、Log、Envelope
默认保留 30 天，由后台任务定时清理；分别使用 `APP__RETENTION_DAYS` 和
`APP__RETENTION_INTERVAL_SECONDS` 配置。

当前版本已具备组织/用户隔离、会话认证、基础 RBAC、管理 API 鉴权、接入限流、
Payload 大小限制、数据保留任务、审计日志、Prometheus 指标和 Sentry Envelope/SDK
兼容接入，并已通过自动化生产验收。

控制台同时提供 Project 创建/归档、独立 DSN Key 一次性生成与轮换、成员角色与
Session 管理、Issue 服务端搜索/批量处理/评论/回归检测、项目级敏感字段清洗与保留
周期，以及告警启停、测试、投递历史和失败重试。Project Key 原文不会写入数据库。
高级工作流包括 Issue Merge/Split、手动 fingerprint、服务/release/时间游标筛选、
48 小时一次性邀请链接、带 24 小时冷静期的项目永久删除、认证附件下载，以及窗口
阈值告警、恢复通知和通知渠道配置检查。P1 已加入 Release/Deployment 健康与版本对比、
JavaScript Source Map 解析、ELF/Mach-O 符号名解析、PDB 安全上传校验、事件重新处理、
HMAC-SHA256 Webhook、值班表和失败后的多级升级投递。

后续演进重点：

1. **协议深度**：PDB 行号解析、performance tracing 和 session replay。
2. **可观测性**：OpenTelemetry trace、outbox 积压和通知失败主动告警。
3. **规模化**：接入层后置 NATS/Kafka；事件明细进入 ClickHouse；PostgreSQL 保留租户、规则和 Issue 元数据。
4. **安全增强**：外部密钥管理、可配置敏感字段 scrubbing、OIDC/SSO 和更细粒度权限策略。

更完整的边界和演进决策见 [`docs/architecture.md`](docs/architecture.md)。
可执行的发布门禁、容量阈值和灾备流程见
[`docs/production-acceptance.md`](docs/production-acceptance.md)。

## 生产运行

```bash
cp .env.production.example .env.production
docker compose --env-file .env.production -f docker-compose.production.yml up -d --build
scripts/production-acceptance.sh
```

生产镜像均使用非 root 用户，并通过数据库、后端、前端三级健康检查控制启动顺序。

## Web 控制台

前端位于 `frontend/`，提供项目概览、Service 目录、结构化 Logs、Event 流、组合
筛选、Issue 状态处理、Fix 修复上下文、Release/符号文件管理、Webhook 与值班升级、
事件时间线和原始 Envelope Item 监控。

```bash
cd frontend
cp .env.example .env.local
npm install
npm run dev
```

默认访问 `http://localhost:3000`，并连接 `http://127.0.0.1:8080`。
