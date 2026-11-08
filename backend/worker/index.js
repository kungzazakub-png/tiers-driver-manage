// Cloudflare Worker: receives hardware-ID reports from the app and keeps them in KV until the GitHub Action collects them.
// No GitHub credential lives here: the Action pulls with a shared read key (secret READ_KEY).
// POST /report   {"v":1,"devices":[{"ids":["PCI\\VEN_..."],"class":"NET","name":"..."}]}   (public, validated, rate-limited)
// GET  /pending  -> {"items":[{"key":"rep:..","devices":[...]}]}                            (Authorization: Bearer READ_KEY)
// POST /ack      {"keys":["rep:.."]}  deletes collected reports                              (Authorization: Bearer READ_KEY)
const ID = /^[A-Za-z0-9_&\\\-.#{}]{1,200}$/;
const MAX_BODY = 16384, MAX_DEVICES = 50, PER_HOUR = 10;

/** Returns the clean device list, or null when the body is not a valid report. Pure function: tested in index.test.mjs. */
export function validate(data) {
  if (!data || data.v !== 1 || !Array.isArray(data.devices)) return null;
  const devices = [];
  for (const d of data.devices.slice(0, MAX_DEVICES)) {
    if (!d || !Array.isArray(d.ids)) continue;
    const ids = d.ids.filter((i) => typeof i === 'string' && ID.test(i)).slice(0, 4);
    if (!ids.length) continue;
    const clean = (s, n) => String(s ?? '').replace(/[\u0000-\u001f`]/g, '').slice(0, n);
    devices.push({ ids, class: clean(d.class, 32).replace(/[^A-Za-z0-9]/g, ''), name: clean(d.name, 80) });
  }
  return devices.length ? devices : null;
}

async function sha256(text) {
  const b = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, '0')).join('');
}

/** Constant-time string comparison (the read key must not leak through timing). */
export function safeEqual(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string' || a.length !== b.length || !a.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}
export const authorized = (req, env) => safeEqual((req.headers.get('Authorization') || '').replace(/^Bearer /, ''), env.READ_KEY || '');

const json = (status, obj) => new Response(JSON.stringify(obj), { status, headers: { 'content-type': 'application/json' } });
const reply = (status, text) => new Response(text, { status, headers: { 'content-type': 'text/plain' } });

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (!env.RATE) return reply(503, 'storage not configured');

    if (url.pathname === '/pending' && req.method === 'GET') {
      if (!authorized(req, env)) return reply(401, 'unauthorized');
      const list = await env.RATE.list({ prefix: 'rep:', limit: 100 });
      const items = [];
      for (const k of list.keys) { const v = await env.RATE.get(k.name); if (v) items.push({ key: k.name, devices: JSON.parse(v) }); }
      return json(200, { items });
    }
    if (url.pathname === '/ack' && req.method === 'POST') {
      if (!authorized(req, env)) return reply(401, 'unauthorized');
      let data; try { data = await req.json(); } catch { return reply(400, 'bad json'); }
      const keys = (Array.isArray(data.keys) ? data.keys : []).filter((k) => typeof k === 'string' && k.startsWith('rep:')).slice(0, 100);
      await Promise.all(keys.map((k) => env.RATE.delete(k)));
      return json(200, { deleted: keys.length });
    }

    if (url.pathname !== '/report') return reply(404, 'not found');
    if (req.method !== 'POST') return reply(405, 'POST only');
    if (!(req.headers.get('content-type') || '').includes('application/json')) return reply(415, 'JSON only');
    const raw = await req.text();
    if (raw.length > MAX_BODY) return reply(413, 'too large');
    let data;
    try { data = JSON.parse(raw); } catch { return reply(400, 'bad json'); }
    const devices = validate(data);
    if (!devices) return reply(400, 'no valid devices');

    // per-IP limit (the IP is only stored as a hash for one hour)
    const key = 'rl:' + (await sha256(req.headers.get('CF-Connecting-IP') || 'unknown'));
    const n = parseInt((await env.RATE.get(key)) || '0', 10);
    if (n >= PER_HOUR) return reply(429, 'too many reports, try later');
    await env.RATE.put(key, String(n + 1), { expirationTtl: 3600 });
    // hardware IDs that were already reported in the last 7 days are not stored again
    const fresh = [];
    for (const d of devices) {
      const k = 'seen:' + (await sha256(d.ids[0].toUpperCase()));
      if (await env.RATE.get(k)) continue;
      await env.RATE.put(k, '1', { expirationTtl: 7 * 86400 });
      fresh.push(d);
    }
    if (!fresh.length) return reply(202, 'already known');
    await env.RATE.put(`rep:${Date.now()}:${crypto.randomUUID()}`, JSON.stringify(fresh), { expirationTtl: 7 * 86400 });
    return reply(202, 'accepted');
  },
};