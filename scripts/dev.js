// Serves site/ with live reload. Edits under site/ reload the browser, and edits
// under apps/tauri/ui/ rebuild the demo first.
//
//   bun scripts/dev.js [port]

import { $ } from 'bun';
import { watch } from 'node:fs';
import { join, normalize, sep } from 'node:path';

const root = join(import.meta.dir, '..');
const site = join(root, 'site');
const firstPort = Number(Bun.argv[2]) || 8000;

// Only the top window reloads, so the demo iframe doesn't reload twice.
const reloadClient = `<script>
if (window.top === window) {
  new WebSocket('ws://' + location.host + '/__reload').onmessage = () => location.reload();
}
</script>`;

const inject = new HTMLRewriter().on('body', {
  element: body => body.append(reloadClient, { html: true }),
});

const buildDemo = () => $`pwsh -NoProfile -File ${join(root, 'scripts/build-demo.ps1')}`.nothrow();

function serve(port) {
  return Bun.serve({
    port,
    hostname: '127.0.0.1',
    development: true,

    async fetch(req, server) {
      const { pathname } = new URL(req.url);
      if (pathname === '/__reload' && server.upgrade(req)) return;

      const path = normalize(join(site, decodeURIComponent(pathname)));
      if (path !== site && !path.startsWith(site + sep)) return new Response('Forbidden', { status: 403 });

      let file = Bun.file(path);
      if ((await file.stat().catch(() => null))?.isDirectory()) file = Bun.file(join(path, 'index.html'));
      if (!(await file.exists())) return new Response('Not found', { status: 404 });

      const res = new Response(file, { headers: { 'Cache-Control': 'no-store' } });
      return file.type.startsWith('text/html') ? inject.transform(res) : res;
    },

    websocket: {
      open: ws => ws.subscribe('reload'),
      message() {},
    },
  });
}

// The next port is tried when one is taken, so the printed link is always right.
function listen(port) {
  try {
    return serve(port);
  } catch (err) {
    if (err.code !== 'EADDRINUSE' || port > firstPort + 20) throw err;
    return listen(port + 1);
  }
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
const server = listen(firstPort);

watch(site, { recursive: true }, debounce(() => server.publish('reload', 'reload')));

let building = false;
watch(
  join(root, 'apps/tauri/ui'),
  { recursive: true },
  debounce(async () => {
    if (building) return;
    building = true;
    await buildDemo();
    building = false;
  }),
);

console.log(`\n  Site dev server running at ${server.url}\n`);
