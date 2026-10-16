// A tiny HTTPS app for the TLS lab: GET /item calls the sync hook (which
// talks to Redis over rediss:// with a password) and answers {"title":"true"}.
// The scenario compares that with pg_stat_ssl.ssl for its own PostgreSQL
// connection, so the check passes only if HTTPS, rediss, and PostgreSQL TLS
// all work. Env: PORT (8443), CERT, KEY, CAWIT_SYNC_URL, NODE_EXTRA_CA_CERTS.
const fs = require('fs');
const https = require('https');
const cawit = require('/hook');

const server = https.createServer(
  { cert: fs.readFileSync(process.env.CERT), key: fs.readFileSync(process.env.KEY) },
  async (req, res) => {
    try {
      if (req.method === 'GET' && req.url === '/item') {
        await cawit.syncPoint('refill:after-read', 'item');
        res.writeHead(200, { 'content-type': 'application/json', connection: 'close' });
        res.end(JSON.stringify({ title: 'true' }));
        return;
      }
      res.writeHead(404, { connection: 'close' });
      res.end();
    } catch (e) {
      res.writeHead(500, { 'content-type': 'text/plain', connection: 'close' });
      res.end(String(e && e.message));
    }
  },
);
server.listen(Number(process.env.PORT || 8443), '127.0.0.1', () => {
  console.log(`tls lab app on ${server.address().port}, hook enabled=${cawit.enabled}`);
});
