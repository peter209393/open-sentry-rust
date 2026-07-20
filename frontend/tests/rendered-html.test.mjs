import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);
  return worker.fetch(new Request("http://localhost/", { headers:{accept:"text/html"} }), { ASSETS:{fetch:async()=>new Response("Not found",{status:404})} }, {waitUntil(){},passThroughOnException(){}});
}

test("production bundle renders the authenticated console shell", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);
  assert.equal(response.headers.get("x-frame-options"), "DENY");
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  const html = await response.text();
  assert.match(html, /<html lang="zh-CN">/);
  assert.match(html, /<title>Open Sentry · Error Intelligence<\/title>/);
  assert.match(html, /正在验证会话/);
  assert.doesNotMatch(html, /Your site is taking shape|Building your site/);
});

test("console source contains login, dashboards, filters and issue actions", async () => {
  const page = await readFile(new URL("../app/page.tsx", import.meta.url), "utf8");
  for (const contract of [
    "/api/auth/me", "/api/auth/login", "/api/auth/logout", "登录监控工作区",
    "最近 24 小时", "事件分布", "serviceFilter", "environmentFilter",
    "startFix", "resolveIssue", "ignoreIssue", "结构化 SDK LOGS",
    "/api/audit-logs", "/api/runtime-config", "createAlertRule", "SDK 接入",
  ]) assert.ok(page.includes(contract), `missing product contract: ${contract}`);
});

test("console exposes P0 project, member, DSN and alert operations", async () => {
  const page = await readFile(new URL("../app/page.tsx", import.meta.url), "utf8");
  for (const contract of [
    "项目管理", "成员权限", "生成新 Key", "解决当前页", "最近投递", "团队协作",
    "/api/issues/batch", "/api/notifications/${n.id}/retry",
    "mergeIssue", "splitIssue", "生成邀请链接", "申请永久删除",
    "窗口阈值", "检查渠道", "/api/envelope-items/${item.id}/download",
  ]) assert.ok(page.includes(contract), `missing P0 product contract: ${contract}`);
});

test("backend proxy preserves session cookies in both directions", async () => {
  const route = await readFile(new URL("../app/backend-api/[...path]/route.ts", import.meta.url), "utf8");
  assert.match(route, /request\.headers\.get\("cookie"\)/);
  assert.match(route, /response\.headers\.get\("set-cookie"\)/);
  assert.match(route, /headers\.set\("set-cookie"/);
});
