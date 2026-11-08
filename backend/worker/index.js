// Cloudflare Worker: receives hardware-ID reports from the app and files them as a GitHub issue (least privilege: the token can only write issues).
// POST /report  {"v":1,"devices":[{"ids":["PCI\\VEN_..."],"class":"NET","name":"..."}]}
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

const reply = (status, text) => new Response(text, { status, headers: { 'content-type': 'text/plain' } });

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (url.pathname !== '/report') return reply(404, 'not found');
    if (req.method !== 'POST') return reply(405, 'POST only');
    if (!(req.headers.get('content-type') || '').includes('application/json')) return reply(415, 'JSON only');
    const raw = await req.text();
    if (raw.length > MAX_BODY) return reply(413, 'too large');
    let data;
    try { data = JSON.parse(raw); } catch { return reply(400, 'bad json'); }
    const devices = validate(data);
    if (!devices) return reply(400, 'no valid devices');

    // per-IP limit (needs the RATE KV namespace; the IP is only stored as a hash for one hour)
    if (env.RATE) {
      const key = 'rl:' + (await sha256(req.headers.get('CF-Connecting-IP') || 'unknown'));
      const n = parseInt((await env.RATE.get(key)) || '0', 10);
      if (n >= PER_HOUR) return reply(429, 'too many reports, try later');
      await env.RATE.put(key, String(n + 1), { expirationTtl: 3600 });
    }
    // hardware IDs that were already reported in the last 7 days are not filed again
    const fresh = [];
    for (const d of devices) {
      const k = 'seen:' + (await sha256(d.ids[0].toUpperCase()));
      if (env.RATE && (await env.RATE.get(k))) continue;
      if (env.RATE) await env.RATE.put(k, '1', { expirationTtl: 7 * 86400 });
      fresh.push(d);
    }
    if (!fresh.length) return reply(202, 'already known');

    const body = '```json\n' + JSON.stringify({ v: 1, devices: fresh }) + '\n```';
    const r = await fetch(`https://api.github.com/repos/${env.GH_REPO}/issues`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${env.GH_TOKEN}`, 'User-Agent': 'tiers-report-worker', Accept: 'application/vnd.github+json', 'content-type': 'application/json' },
      body: JSON.stringify({ title: `HWID report: ${fresh.length} device(s)`, body }),
    });
    return r.ok ? reply(202, 'accepted') : reply(502, 'could not file the report');
  },
};
