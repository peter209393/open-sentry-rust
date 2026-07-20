import { expect, test } from "@playwright/test";

test("owner can login and reach production management surfaces", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("邮箱").fill(process.env.E2E_ADMIN_EMAIL ?? "admin@example.com");
  await page.getByLabel("密码").fill(process.env.E2E_ADMIN_PASSWORD ?? "change-me");
  await page.getByRole("button", { name: "登录" }).click();

  await expect(page.getByRole("heading", { name: "运行概览" })).toBeVisible();
  await expect(page.getByText("系统正常")).toBeVisible();

  await page.getByRole("button", { name: "项目管理" }).click();
  await expect(page.getByRole("heading", { name: "项目与数据边界" })).toBeVisible();
  await page.getByRole("button", { name: "成员权限" }).click();
  await expect(page.getByRole("heading", { name: "成员与角色" })).toBeVisible();
  await page.getByRole("button", { name: "告警规则" }).click();
  await expect(page.getByRole("heading", { name: "创建告警" })).toBeVisible();
  await page.getByRole("button", { name: "Releases" }).click();
  await expect(page.getByRole("heading", { name: "版本与部署基线" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "调试文件" })).toBeVisible();
  await page.getByRole("button", { name: "集成与值班" }).click();
  await expect(page.getByRole("heading", { name: "Webhook 端点" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "值班与升级策略" })).toBeVisible();
  await page.getByRole("button", { name: "项目设置" }).click();
  await expect(page.getByRole("heading", { name: "接入密钥" })).toBeVisible();
});
