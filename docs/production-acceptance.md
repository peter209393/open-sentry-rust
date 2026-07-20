# 生产验收标准

本文定义当前 error/event monitoring 范围的发布门禁。Performance tracing、Session
Replay、Source Map 符号化和 ClickHouse 规模化属于架构文档明确列出的后续能力，
不计入当前版本发布范围。

## 功能与协议

- Project Key 必须相互隔离；邀请令牌、Merge/Split、附件授权下载和删除冷静期必须
  通过真实 API 验证。
- 阈值规则达到窗口计数时触发，Resolve 应生成去重的恢复通知；渠道检查必须返回
  可执行的配置错误。

- Sentry DSN、JSON Store 和现代 Envelope 接入可用。
- Gzip、Zlib、Event、Transaction、Attachment、Log 和 Panic 覆盖测试通过。
- Event、Payload、Envelope、DSN 语义与代码模型一致。
- SDK fingerprint 优先；默认 fingerprint 使用异常类型与稳定 in-app frame。
- Issue 聚合、状态、Fix、告警 Outbox、服务、日志和筛选可用。

证据：`cargo test --all-targets`、`cargo run --bin sentry-smoke -- all`。

## 安全与租户

- 管理 API 默认要求数据库会话；Organization/Project/Issue 查询执行租户校验。
- 密码使用 Argon2；Session 和 DSN Key 仅存哈希；Cookie 为 HttpOnly、SameSite，
  生产环境强制 Secure。
- 生产启动拒绝默认管理员密码、默认 Metrics Key 和非安全 Cookie。
- Payload 敏感字段递归清洗；请求体、解压体、Item 和 Event 均有硬限制。
- 前端启用禁止嵌入、MIME 嗅探、权限和 Referrer 安全响应头。
- 前端运行依赖 `npm audit --audit-level=high` 必须通过。

## 可靠性与运维

- 事件、Issue 聚合和通知 Outbox 在同一事务内提交。
- 接入层按凭证严格限流，429 必须包含 `Retry-After`。
- `/health` 验证数据库连接；Compose 在依赖健康后才启动下游服务。
- `/metrics` 使用独立 Bearer Key，提供接入、限流、认证失败和保留清理计数。
- 保留 Worker 自动清理过期 Event、Log、Envelope 和 Session。
- 身份、Issue、Fix 和告警变更写入组织级审计日志。

## 容量门禁

本地基准的发布最低门槛：200 请求、20 并发、错误率不高于 1%、p95 不高于
500ms。生产硬件应使用相同工具重新建立环境专属基线：

```bash
LOAD_REQUESTS=200 LOAD_CONCURRENCY=20 LOAD_MAX_P95_MS=500 \
  cargo run --release --bin ingest-load
```

2026-07-20 本机验收结果：200/200 成功，506.5 req/s，p50 21ms，p95 166ms，
p99 226ms。

## 备份与恢复

建议至少每小时执行 `scripts/backup-postgres.sh`，目标 RPO 为 1 小时。每次备份
生成 PostgreSQL Custom Dump 和 SHA-256。每个发布周期至少恢复到独立数据库：

```bash
scripts/backup-postgres.sh
RESTORE_DATABASE=open_sentry_restore CONFIRM_RESTORE=open_sentry_restore \
  scripts/restore-postgres.sh backups/open_sentry_TIMESTAMP.dump
```

恢复脚本禁止默认覆盖生产库。验收必须核对 Projects、Events 和 Users 行数。

## 发布门禁

```bash
scripts/production-acceptance.sh
```

该命令依次执行 Rust 格式、全部测试、Clippy、前端测试、Lint、依赖审计、健康
检查、并发基准，以及前后端镜像构建。任意一步失败即阻止发布。
