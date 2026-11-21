// Serves site/ with live reload. Edits under site/ reload the browser, and edits
// under apps/tauri/ui/ rebuild the demo first.
//
//   bun scripts/dev.js [port]

import { spawn } from 'node:child_process';
import { createReadStream, watch } from 'node:fs';
import { readFile, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const site = join(root, 'site');
const tauriUi = join(root, 'apps', 'tauri', 'ui');
const firstPort = Number(process.argv[2]) || 8000;

const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
  '.ico': 'image/x-icon',
  '.mp3': 'audio/mpeg',
  '.ogg': 'audio/ogg',
  '.woff2': 'font/woff2',
};

// Only the top window reloads, so the demo iframe doesn't reload twice.
const reloadClient = `<script>
if (window.top === window) {
  new EventSource('/__reload').onmessage = () => location.reload();
}
</script>`;

const clients = new Set();

function reload() {
  for (const res of clients) res.write('data: reload\n\n');
}

function buildDemo() {
  return new Promise(done => {
    const ps = spawn('pwsh', ['-NoProfile', '-File', join(root, 'scripts', 'build-demo.ps1')], {
      stdio: 'inherit',
    });
    ps.on('error', err => {
      console.error(`build-demo failed to start: ${err.message}`);
      done();
    });
    ps.on('exit', done);
  });
}

async function handle(req, res) {
  const url = new URL(req.url, 'http://localhost');

  if (url.pathname === '/__reload') {
    res.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    });
    res.write(': connected\n\n');
    clients.add(res);
    req.on('close', () => clients.delete(res));
    return;
  }

  let file = normalize(join(site, decodeURIComponent(url.pathname)));
  if (file !== site && !file.startsWith(site + sep)) {
    res.writeHead(403).end();
    return;
  }

  try {
    if ((await stat(file)).isDirectory()) file = join(file, 'index.html');
    const info = await stat(file);
    const type = types[extname(file).toLowerCase()] ?? 'application/octet-stream';
    const headers = { 'Content-Type': type, 'Cache-Control': 'no-store' };

    if (type.startsWith('text/html')) {
      const html = await readFile(file, 'utf8');
      const body = html.includes('</body>')
        ? html.replace('</body>', `${reloadClient}</body>`)
        : html + reloadClient;
      res.writeHead(200, headers).end(body);
      return;
    }

    res.writeHead(200, { ...headers, 'Content-Length': info.size });
    createReadStream(file).pipe(res);
  } catch {
    res.writeHead(404, { 'Content-Type': 'text/plain' }).end('Not found');
  }
}

function listen(server, port) {
  return new Promise((ok, fail) => {
    server.once('error', fail);
    server.listen(port, 'localhost', () => ok(port));
  }).catch(err => {
    if (err.code !== 'EADDRINUSE' || port > firstPort + 20) throw err;
    return listen(server, port + 1);
  });
}

// Editors fire several events per save, so changes settle for 100 ms first.
function debounce(fn, ms = 100) {
  let timer;
  return () => {
    clearTimeout(timer);
    timer = setTimeout(fn, ms);
  };
}

await buildDemo();

const server = createServer(handle);
const port = await listen(server, firstPort);

watch(site, { recursive: true }, debounce(reload));

let building = false;
watch(
  tauriUi,
  { recursive: true },
  debounce(async () => {
    if (building) return;
    building = true;
    await buildDemo();
    building = false;
  }),
);

console.log(`\n  Site dev server running at http://localhost:${port}/\n`);
