# 架构设计与演进

## 目标与边界

当前版本定位为可生产部署的 error/event monitoring 平台，而不是一次性复制 Sentry
的全部能力。核心闭环包括：接收事件、可靠落库、按 fingerprint 聚合 Issue、查询和
改变状态、根据规则触发通知，并通过认证控制台完成项目、服务、日志和事件管理。

当前边界暂不包含完整的 performance tracing、session replay、source map 和复杂搜索
DSL。这些属于后续协议深度与规模化演进，不影响当前错误监控、日志观测和运维闭环。

## 领域模型

- **Project**：事件隔离、DSN 和告警配置边界。
- **Event**：不可变的单次错误事实，保留环境、release、tags、contexts、exception。
- **Issue**：相似 Event 的聚合；记录首次/最近发生时间、次数和处理状态。
- **AlertRule**：匹配 level、环境、message substring，并带冷却窗口。
- **NotificationOutbox**：通知任务及重试状态，保证数据库提交后告警不会凭空丢失。

fingerprint 优先使用 SDK 显式组件（支持 `{{ default }}`），否则使用 level、异常类型和最多
5 个稳定 in-app stack frame；没有 stacktrace 时才回退到规范化 message。所有组件最终
计算 SHA-256，避免订单号等动态消息造成 Issue 碎片化。

## 一致性与失败策略

Ingest 在单个事务中 upsert Issue、插入 Event、锁定匹配规则并写 outbox。API 返回 `202 Accepted` 表示数据已持久化。Worker 通过 `FOR UPDATE SKIP LOCKED` 领取任务；失败后指数退避，达到上限进入 failed，供运维查询和人工重放。

通知语义为 at-least-once。极端情况下进程在外部发送成功、更新 outbox 之前崩溃，会重复通知。若渠道支持 idempotency key，应使用 outbox id；否则邮件/TG 文案应带 event id 便于识别。

## 容量分界建议

## 控制台认证与租户边界

控制台用户隶属于 Organization，Project 继承 Organization 隔离边界。密码使用
Argon2 哈希；登录后生成高熵随机会话令牌，数据库仅保存令牌 SHA-256 摘要，浏览器
通过 `HttpOnly; SameSite=Strict` Cookie 持有原始令牌。除健康检查、登录和 SDK
接入外，管理 API 必须先验证会话，并再次校验用户与目标 Project/Issue 的组织关系。
生产环境必须覆盖 bootstrap 管理员密码，并由边缘 TLS 层负责 HTTPS。

## 运行保障

- SDK 接入按凭证执行固定窗口限流，超限返回 429 与 Retry-After，避免 Log Storm
  反向击穿监控系统。
- `/metrics` 使用独立 Bearer 凭证保护，输出接入请求、接收量、限流量、认证失败和
  保留清理量等 Prometheus Counter。
- 保留任务周期清理过期 Event、Log、Envelope 和 Session；默认窗口为 30 天。
- 登录、退出、Issue 状态、Fix 和告警规则变更写入不可变审计日志，并限定在
  Organization 范围内查询。
- 每个 Project 使用独立接入 Key；服务端仅保存 SHA-256 摘要。新 Key 只在创建响应
  中展示一次，轮换期间可并存，吊销立即停止接入。
- Project 可覆盖默认数据保留周期和敏感字段清洗列表。已解决 Issue 再次收到同一
  fingerprint 时自动重新打开并记录回归时间。
- Issue 支持 Merge/Split 和显式 fingerprint 调整；所有操作继续受组织边界和 RBAC
  约束。附件只能通过认证下载端点读取，并强制 `private, no-store`。
- 项目永久删除采用 owner 确认与 24 小时冷静期。成员邀请令牌只存摘要，48 小时后
  过期且只能接受一次。
- 告警可按时间窗口计数触发，并在 Issue 恢复时通过带去重键的 outbox 发送恢复通知。

- 低于约 100 events/s：当前 PostgreSQL 方案足够，先做分区、批量清理和索引治理。
- 数百到数千 events/s：接入 API 与 worker 独立部署，引入消息队列削峰。
- 大规模日志分析：原始 Event 进入 ClickHouse，对象存储保存附件；PostgreSQL 只保留控制面和 Issue 投影。

这些数值是工程起点而非硬限制，应以 payload 大小、查询形态、磁盘 IOPS 和压测结果决定迁移时机。

## 后续演进建议

1. 接入 OIDC/SSO，并在现有 owner/admin/member RBAC 上增加更细粒度权限策略。
2. 实现 source map、release health、performance tracing 和 session replay 协议能力。
3. 增加按月分区、可配置敏感信息清洗以及对象存储附件归档。
4. 增加趋势窗口规则，例如“5 分钟超过 20 次”，并将 rule evaluation 从同步 ingest 移到流处理 worker。
5. 在容量达到分界点后引入消息队列与 ClickHouse，拆分接入、查询和通知 worker。
