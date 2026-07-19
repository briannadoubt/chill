const securityHeaders = {
  "content-security-policy": "default-src 'self'; connect-src 'self' https: http://localhost:* http://127.0.0.1:*; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
  "referrer-policy": "no-referrer",
  "x-content-type-options": "nosniff",
  "x-frame-options": "DENY",
};

function secured(response) {
  const headers = new Headers(response.headers);
  for (const [name, value] of Object.entries(securityHeaders)) headers.set(name, value);
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

function json(body, status) {
  return secured(Response.json(body, {
    status,
    headers: { "cache-control": "no-store" },
  }));
}

function empty(status) {
  return secured(new Response(null, {
    status,
    headers: { "cache-control": "no-store" },
  }));
}

async function exchangeSession(request, env) {
  if (request.method !== "POST") {
    return json({ error: "method not allowed" }, 405);
  }
  const email = request.headers.get("oai-authenticated-user-email");
  if (!email) {
    return json({ error: "private site authentication required" }, 401);
  }
  if (!env.CHILL_API_ORIGIN || !env.CHILL_SITES_AUTH_SECRET) {
    return json({ error: "authentication is not configured" }, 503);
  }
  const apiBase = env.CHILL_API_ORIGIN.replace(/\/+$/, "");
  let upstream;
  try {
    upstream = await fetch(`${apiBase}/v1/auth/sites-session`, {
      method: "POST",
      headers: {
        "accept": "application/json",
        "content-type": "application/json",
        "x-chill-sites-auth": env.CHILL_SITES_AUTH_SECRET,
      },
      body: JSON.stringify({ email }),
    });
  } catch {
    return json({ error: "Chill authentication is unavailable" }, 502);
  }
  if (!upstream.ok) {
    return json({ error: "authenticated user is not authorized for Chill" }, upstream.status === 401 ? 401 : 502);
  }
  const issued = await upstream.json();
  if (typeof issued?.credential !== "string" || typeof issued?.expiresAt !== "string") {
    return json({ error: "Chill authentication returned an invalid response" }, 502);
  }
  return json({ apiBase, credential: issued.credential, expiresAt: issued.expiresAt }, 201);
}

async function relayTelemetry(request, env) {
  if (request.method !== "POST") {
    return json({ error: "method not allowed" }, 405);
  }
  if (!request.headers.get("oai-authenticated-user-email")) {
    return json({ error: "private site authentication required" }, 401);
  }
  if (!env.CHILL_API_ORIGIN || !env.CHILL_CONSOLE_SDK_KEY) {
    return json({ error: "telemetry is not configured" }, 503);
  }
  if (!request.headers.get("content-type")?.toLowerCase().startsWith("application/json")) {
    return json({ error: "content type must be application/json" }, 415);
  }
  const declaredLength = Number(request.headers.get("content-length") ?? "0");
  if (Number.isFinite(declaredLength) && declaredLength > 131_072) {
    return json({ error: "telemetry payload is too large" }, 413);
  }
  const body = await request.text();
  if (new TextEncoder().encode(body).byteLength > 131_072) {
    return json({ error: "telemetry payload is too large" }, 413);
  }
  let payload;
  try {
    payload = JSON.parse(body);
  } catch {
    return json({ error: "telemetry payload must be valid JSON" }, 400);
  }
  if (!payload || !Array.isArray(payload.resourceLogs) || payload.resourceLogs.length === 0) {
    return json({ error: "telemetry payload is invalid" }, 400);
  }

  const apiBase = env.CHILL_API_ORIGIN.replace(/\/+$/, "");
  let upstream;
  try {
    upstream = await fetch(`${apiBase}/v1/logs`, {
      method: "POST",
      headers: {
        "authorization": `Bearer ${env.CHILL_CONSOLE_SDK_KEY}`,
        "content-type": "application/json",
        "x-chill-schema-version": "1.0.0",
      },
      body,
    });
  } catch {
    return json({ error: "Chill telemetry is unavailable" }, 502);
  }
  if (!upstream.ok) {
    return json({ error: "Chill telemetry rejected the payload" }, 502);
  }
  return empty(202);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/api/session") return exchangeSession(request, env);
    if (url.pathname === "/v1/logs") return relayTelemetry(request, env);

    let response = await env.ASSETS.fetch(request);
    if (response.status === 404 && request.method === "GET" && !url.pathname.includes(".")) {
      response = await env.ASSETS.fetch(new Request(new URL("/index.html", url), request));
    }
    return secured(response);
  },
};
