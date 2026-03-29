const DEFAULT_POCKETBASE_URL = "https://puckduel.dano.win/";
const ROOM_TTL_SECS = 5 * 60;

export interface RoomRecord {
  id: string;
  node_addr: string;
}

interface PbList<T> {
  items: T[];
}

function pocketBaseApiBase(): string {
  const raw = (import.meta.env.PUBLIC_POCKETBASE_URL as string | undefined)?.trim();
  const base = (raw && raw.length > 0 ? raw : DEFAULT_POCKETBASE_URL).replace(/\/+$/, "");
  return base.endsWith("/api") ? base : `${base}/api`;
}

function pocketBaseToken(): string | null {
  const token = (import.meta.env.PUBLIC_POCKETBASE_TOKEN as string | undefined)?.trim();
  return token && token.length > 0 ? token : null;
}

function headers(json: boolean): HeadersInit {
  const token = pocketBaseToken();
  const h: Record<string, string> = {};
  if (json) h["Content-Type"] = "application/json";
  if (token) h["Authorization"] = `Bearer ${token}`;
  return h;
}

function nowUnixSecs(): number {
  return Math.floor(Date.now() / 1000);
}

async function requestJson<T>(url: string): Promise<T> {
  const resp = await fetch(url, { headers: headers(false) });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`PocketBase request failed (${resp.status}): ${text || resp.statusText}`);
  }
  return (await resp.json()) as T;
}

export function generateRoomCode(): string {
  return Math.floor(Math.random() * 10000).toString().padStart(4, "0");
}

export async function cleanupExpiredRooms(): Promise<void> {
  const api = pocketBaseApiBase();
  const url = new URL(`${api}/collections/rooms/records`);
  const qs = new URLSearchParams({
    filter: `expires_at <= ${nowUnixSecs()}`,
    fields: "id",
    perPage: "200",
  });
  url.search = qs.toString();

  const list = await requestJson<PbList<{ id: string }>>(url.toString());
  await Promise.all(
    list.items.map((r) =>
      fetch(`${api}/collections/rooms/records/${r.id}`, {
        method: "DELETE",
        headers: headers(false),
      }).catch(() => undefined),
    ),
  );
}

export async function createRoom(code: string, nodeAddr: string): Promise<string> {
  const api = pocketBaseApiBase();
  const resp = await fetch(`${api}/collections/rooms/records`, {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({
      code,
      node_addr: nodeAddr,
      expires_at: nowUnixSecs() + ROOM_TTL_SECS,
    }),
  });

  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`PocketBase create failed (${resp.status}): ${text || resp.statusText}`);
  }

  const body = (await resp.json()) as { id: string };
  return body.id;
}

export async function findRoomByCode(code: string, timeoutMs = 30_000): Promise<RoomRecord | null> {
  const api = pocketBaseApiBase();
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    const now = nowUnixSecs();
    const url = new URL(`${api}/collections/rooms/records`);
    const qs = new URLSearchParams({
      filter: `code = \"${code}\" && expires_at > ${now}`,
      perPage: "1",
      sort: "-expires_at",
      fields: "id,node_addr",
    });
    url.search = qs.toString();

    try {
      const rows = await requestJson<PbList<RoomRecord>>(url.toString());
      if (rows.items.length > 0) {
        return rows.items[0];
      }
    } catch {
      // Keep polling until timeout to handle transient network issues.
    }

    await new Promise((resolve) => setTimeout(resolve, 1000));
  }

  return null;
}

export async function updateRoomJoinerAddr(roomId: string, joinerAddr: string): Promise<void> {
  const api = pocketBaseApiBase();
  const resp = await fetch(`${api}/collections/rooms/records/${roomId}`, {
    method: "PATCH",
    headers: headers(true),
    body: JSON.stringify({ joiner_addr: joinerAddr }),
  });

  if (!resp.ok && resp.status !== 404) {
    const text = await resp.text();
    throw new Error(`PocketBase patch failed (${resp.status}): ${text || resp.statusText}`);
  }
}

export async function deleteRoom(roomId: string): Promise<void> {
  const api = pocketBaseApiBase();
  const resp = await fetch(`${api}/collections/rooms/records/${roomId}`, {
    method: "DELETE",
    headers: headers(false),
  });

  if (!resp.ok && resp.status !== 404) {
    const text = await resp.text();
    throw new Error(`PocketBase delete failed (${resp.status}): ${text || resp.statusText}`);
  }
}
