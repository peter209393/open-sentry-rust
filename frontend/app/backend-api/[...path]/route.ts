const BACKEND_API_URL = process.env.BACKEND_API_URL ?? "http://127.0.0.1:8080";

type RouteContext = { params: Promise<{ path: string[] }> };

async function proxy(request: Request, context: RouteContext) {
  const { path } = await context.params;
  const incoming = new URL(request.url);
  const target = new URL(`/${path.join("/")}${incoming.search}`, BACKEND_API_URL);
  const response = await fetch(target, {
    method: request.method,
    headers: {
      ...(request.headers.get("content-type") ? { "content-type": request.headers.get("content-type")! } : {}),
      ...(request.headers.get("cookie") ? { cookie: request.headers.get("cookie")! } : {}),
    },
    body: ["GET", "HEAD"].includes(request.method) ? undefined : await request.text(),
  });
  const headers = new Headers({ "content-type": response.headers.get("content-type") ?? "application/json" });
  const setCookie = response.headers.get("set-cookie");
  if (setCookie) headers.set("set-cookie", setCookie);
  headers.set("cache-control", "no-store");
  headers.set("x-content-type-options", "nosniff");
  return new Response(response.body, {
    status: response.status,
    headers,
  });
}

export const GET = proxy;
export const PATCH = proxy;
export const POST = proxy;
