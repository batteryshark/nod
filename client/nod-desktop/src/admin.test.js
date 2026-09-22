// @vitest-environment node
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { afterEach, describe, expect, it, vi } from "vitest";
const html = readFileSync("../../server/nod-server/assets/admin.html", "utf8");
const pages = [];
afterEach(() => {
  pages.splice(0).forEach((page) => page.window.close());
});
function page(overrides = {}, { privateHttp = false } = {}) {
  const fixtures = {
    "/summary": {},
    "/users": {
      users: [
        { id: "owner", name: "Owner", subscribed_channel_ids: ["default"] },
      ],
    },
    "/channels": {
      channels: [{ id: "default", name: "Default", emoji: "🔔" }],
    },
    "/devices": { devices: [] },
    "/issuer-tokens": { tokens: [] },
    "/settings": {},
    "/activity": {
      requests: [
        {
          id: "r1",
          title: '<img src=x onerror="alert(1)">',
          channel_id: "default",
          status: "pending",
        },
      ],
      deliveries: [],
      audit: { healthy: true },
    },
    "/users/owner/enrollment-codes": {
      code: "ABCDEFGH",
      expires_at: "2030-01-01T00:00:00Z",
    },
  };
  const fetch = vi.fn(async (url, init) => {
    const path = String(url).replace("/api/v1/admin", "");
    const handler = overrides[path];
    const data = handler ? await handler(init) : fixtures[path];
    return {
      ok: true,
      status: 200,
      text: async () => JSON.stringify(data ?? {}),
    };
  });
  const dom = new JSDOM(html, {
    url: privateHttp
      ? "http://192.168.1.10:8767/admin"
      : "https://nod.test/admin",
    runScripts: "dangerously",
    beforeParse(window) {
      window.fetch = fetch;
      if (privateHttp)
        Object.defineProperty(window.crypto, "randomUUID", {
          value: undefined,
        });
    },
  });
  pages.push(dom);
  return { dom, fetch, get: (id) => dom.window.document.getElementById(id) };
}
async function ready(get) {
  await vi.waitFor(() => expect(get("status").textContent).toBe("Ready"));
}
describe("embedded admin UI", () => {
  it("runs with local assets and renders activity content as text", async () => {
    const { dom, get } = page();
    await ready(get);
    expect(dom.window.document.querySelector("script[src]")).toBeNull();
    expect(get("activityRequests").querySelector("img")).toBeNull();
    expect(get("activityRequests").textContent).toContain("<img src=x");
    expect(get("auditHealth").textContent).toBe("Audit logging healthy");
  });
  it("creates an explicit one-use enrollment link", async () => {
    const { dom, get } = page();
    await ready(get);
    get("deviceEnrollmentUser").value = "owner";
    get("deviceEnrollmentForm").dispatchEvent(
      new dom.window.Event("submit", { cancelable: true }),
    );
    await vi.waitFor(() =>
      expect(get("deviceEnrollmentSecret").querySelector("a")).not.toBeNull(),
    );
    const link = new URL(get("deviceEnrollmentSecret").querySelector("a").href);
    expect(link.protocol).toBe("nod:");
    expect(link.searchParams.get("server")).toBe("https://nod.test");
    expect(link.searchParams.get("code")).toBe("ABCDEFGH");
  });
  it("reuses the attempt identity after an uncertain create without allowing parallel submission", async () => {
    const bodies = [];
    const { dom, get } = page({
      "/test-requests": async (init) => {
        bodies.push(JSON.parse(init.body));
        throw new Error("connection lost");
      },
    });
    await ready(get);
    get("testTitle").value = "Retry test";
    get("testExpires").value = "3600";
    const submit = () =>
      get("testRequestForm").dispatchEvent(
        new dom.window.Event("submit", { cancelable: true }),
      );
    submit();
    submit();
    await vi.waitFor(() =>
      expect(get("testRequestSubmit").disabled).toBe(false),
    );
    submit();
    await vi.waitFor(() => expect(bodies).toHaveLength(2));
    expect(bodies[0].idempotency_key).toBeTruthy();
    expect(bodies[0].idempotency_key).toBe(bodies[1].idempotency_key);
    expect(bodies[0].expires_at).toBe(bodies[1].expires_at);
  });
  it("creates test requests from private HTTP admin hosts without randomUUID", async () => {
    const bodies = [];
    const { dom, get } = page(
      {
        "/test-requests": async (init) => {
          bodies.push(JSON.parse(init.body));
          return { request_id: "lan-test" };
        },
      },
      { privateHttp: true },
    );
    await ready(get);
    get("testTitle").value = "Private LAN test";
    get("testRequestForm").dispatchEvent(
      new dom.window.Event("submit", { cancelable: true }),
    );
    await vi.waitFor(() => expect(bodies).toHaveLength(1));
    expect(bodies[0].idempotency_key).toMatch(/^admin-test-[0-9a-f]{32}$/);
    await vi.waitFor(() =>
      expect(get("testRequestSubmit").disabled).toBe(false),
    );
    expect(get("status").textContent).toContain("Test request created");
  });
  it("offers selected enrollment-link text when private HTTP cannot copy automatically", async () => {
    const { dom, get } = page({}, { privateHttp: true });
    await ready(get);
    get("deviceEnrollmentUser").value = "owner";
    get("deviceEnrollmentForm").dispatchEvent(
      new dom.window.Event("submit", { cancelable: true }),
    );
    await vi.waitFor(() =>
      expect(get("status").textContent).toBe("Enrollment code created"),
    );
    const button = Array.from(
      get("deviceEnrollmentSecret").querySelectorAll("button"),
    ).find((item) => item.textContent === "Copy enrollment link");
    button.click();
    const field = get("deviceEnrollmentSecret").querySelector("textarea");
    expect(field.value).toContain("nod://enroll?");
    expect(field.value).toContain("code=ABCDEFGH");
    expect(field.selectionEnd).toBe(field.value.length);
    expect(get("status").textContent).toContain("Press Ctrl+C or Command+C");
  });
});
