// Create the `articles` collection with one row (id 1) through the Directus
// API, once. Env: BASE (http://127.0.0.1:8055), TOKEN (admin static token).
const BASE = process.env.BASE || 'http://127.0.0.1:8055';
const auth = { authorization: `Bearer ${process.env.TOKEN}`, 'content-type': 'application/json' };

async function call(method, path, body) {
  const res = await fetch(BASE + path, { method, headers: auth, body: body && JSON.stringify(body) });
  const text = await res.text();
  return { status: res.status, body: text ? JSON.parse(text) : null };
}

(async () => {
  const existing = await call('GET', '/collections/articles');
  if (existing.status === 200) {
    console.log('articles already exists');
  } else {
    const r = await call('POST', '/collections', {
      collection: 'articles',
      schema: {},
      meta: {},
      fields: [
        { field: 'id', type: 'integer', meta: { hidden: true }, schema: { is_primary_key: true, has_auto_increment: true } },
        { field: 'title', type: 'string' },
      ],
    });
    if (r.status >= 400) throw new Error(`create collection: ${JSON.stringify(r.body)}`);
    console.log('created articles');
  }
  const one = await call('GET', '/items/articles/1');
  if (one.status !== 200) {
    const r = await call('POST', '/items/articles', { title: 'seed' });
    if (r.status >= 400) throw new Error(`create item: ${JSON.stringify(r.body)}`);
    console.log('created item', r.body.data.id);
  }
})().catch((e) => { console.error(e); process.exit(1); });
