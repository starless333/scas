// npm install --prefix /tmp/scas-browser playwright@1.64.1
// NODE_PATH=/tmp/scas-browser/node_modules node scripts/check-web-browser.cjs
const assert = require('node:assert/strict');
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_PATH || undefined,
    args: ['--no-sandbox', '--enable-unsafe-swiftshader'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 800, height: 800 } });
    page.setDefaultTimeout(30000);
    await page.addInitScript(() => {
      window.scasTestPackets = [];
      const WorkerBase = window.Worker;
      window.Worker = class extends WorkerBase {
        constructor(...args) {
          super(...args);
          this.addEventListener('message', event => {
            try { window.scasTestPackets.push(JSON.parse(event.data)); } catch { /* Non-SCAS worker. */ }
          });
        }
      };
    });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(process.env.SCAS_WEB_URL || 'http://127.0.0.1:8080/web/');
    await page.waitForSelector('#loading', { state: 'detached', timeout: 60000 });
    // A compact workspace starts with preview closed: no MathJax request is needed.
    assert.equal(await page.locator('script[src*="mathjax"]').count(), 0);
    const results = await page.evaluate(async () => {
      const worker = window.scasCreateWorker();
      let generation = 0;
      const request = payload => new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('Worker response timed out')), 15000);
        worker.onerror = event => { clearTimeout(timer); reject(new Error(event.message)); };
        worker.onmessage = event => { clearTimeout(timer); resolve(JSON.parse(event.data)); };
        worker.postMessage(JSON.stringify({ generation: ++generation, ...payload }));
      });
      try {
        const exact = await request({ kind: 'repl', input: '1/3 + 1/6' });
        await request({ kind: 'repl', input: 'f(x) = x^2 - 2' });
        const definition = await request({ kind: 'repl', input: 'f(3)' });
        const error = await request({ kind: 'repl', input: '1 +' });
        const recovery = await request({ kind: 'repl', input: '2 + 2' });
        const plot = await request({ kind: 'plot', expression: 'sin(x)', source: '' });
        const client = await window.scasCreateLspClient();
        const lsp = await client.request('initialize', {});
        client.dispose();
        return { exact, definition, error, recovery, plot, lsp };
      } finally { worker.terminate(); }
    });
    assert.equal(results.exact.error, null);
    assert.match(results.exact.output, /1\/2/);
    assert.match(results.definition.output, /7/);
    assert.ok(results.error.error);
    assert.match(results.recovery.output, /4/);
    assert.ok(results.plot.series[0].samples.length > 10);
    assert.match(results.plot.svg, /<svg/);
    assert.ok(results.lsp.capabilities);
    // Edit the actual canvas editor, run once, then edit again. Preview must
    // remain live without sending another Run or publishing replacement output.
    await page.mouse.click(160, 130);
    await page.keyboard.press('Control+a');
    await page.keyboard.type('print(71);');
    await page.keyboard.press('Control+Enter');
    await page.waitForFunction(() => scasTestPackets.some(p => p.kind === 'run' && p.output === '71\n'));
    const runs = await page.evaluate(() => scasTestPackets.filter(p => p.kind === 'run').length);
    const previews = await page.evaluate(() => scasTestPackets.filter(p => p.kind === 'preview').length);
    await page.mouse.click(160, 130);
    await page.keyboard.press('Control+a');
    await page.keyboard.type('print(92);');
    await page.waitForFunction(count => scasTestPackets.filter(p => p.kind === 'preview').length > count, previews);
    assert.equal(await page.evaluate(() => scasTestPackets.filter(p => p.kind === 'run').length), runs);
    assert.equal(await page.evaluate(() => scasTestPackets.filter(p => p.kind === 'preview').at(-1).output), null);

    // Ctrl+X with an empty selection deletes the complete current line.
    await page.mouse.click(160, 130);
    await page.keyboard.press('Control+a');
    await page.keyboard.type('print(11);');
    await page.keyboard.press('Enter');
    await page.keyboard.type('print(22);');
    await page.keyboard.press('Enter');
    await page.keyboard.type('print(33);');
    const runsBeforeDelete = await page.evaluate(() => scasTestPackets.filter(p => p.kind === 'run').length);
    await page.keyboard.press('Control+x');
    await page.keyboard.press('Control+Enter');
    await page.waitForFunction(count => scasTestPackets.filter(p => p.kind === 'run').length > count, runsBeforeDelete);
    const deleteRun = await page.evaluate(() => scasTestPackets.filter(p => p.kind === 'run').at(-1));
    assert.equal(deleteRun.output, '11\n22\n');

    // Exercise the on-demand renderer and ensure queued typesetting finishes.
    await page.evaluate(() => window.scasShowPreview(
      JSON.stringify([{kind:'display',latex:'\\[ x^2 \\]',span:{line:1}}]),
      'x^2', '', false, 50, 40, 600, 600, devicePixelRatio, false));
    await page.waitForSelector('#math-preview-content mjx-container', { state: 'attached', timeout: 60000 });
    await page.mouse.click(22, 145);
    await page.waitForSelector('#repl-entry-preview:not([hidden])', { state: 'visible' });
    const composer = await page.locator('#repl-entry-preview').boundingBox();
    await page.mouse.click(composer.x + 50, composer.y + composer.height + 15);
    await page.keyboard.type('log(x, e)');
    await page.waitForSelector('#repl-entry-preview mjx-container', { state: 'visible' });
    assert.equal(await page.locator('#repl-entry-preview [data-typeset="true"]').count(), 1);
    await page.keyboard.press('Enter');
    await page.waitForSelector('#repl-history .repl-result-math mjx-container', { state: 'visible' });
    assert.equal(await page.locator('#repl-history .repl-result-math[data-typeset="true"]').count(), 1);
    assert.equal(await page.locator('#repl-history .repl-calculation').last().getAttribute('class'), 'repl-calculation math-ready');
    assert.equal(await page.locator('#repl-history .repl-math-fallback').last().isHidden(), true);
    assert.match(await page.locator('.repl-result-math').last().getAttribute('data-latex'), /\\ln/);

    // Math output must remain visible when both external MathJax origins are
    // unavailable. The text result is the durable answer; typesetting is a
    // progressive enhancement that replaces it only after successful render.
    const offline = await browser.newPage({ viewport: { width: 800, height: 800 } });
    await offline.route(/(?:cdn\.jsdelivr\.net|unpkg\.com)\/.*mathjax/, route => route.abort());
    await offline.goto(process.env.SCAS_WEB_URL || 'http://127.0.0.1:8080/web/');
    await offline.waitForSelector('#loading', { state: 'detached', timeout: 60000 });
    await offline.evaluate(async () => {
      const worker = window.scasCreateWorker();
      const result = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('Offline REPL response timed out')), 15000);
        worker.onerror = event => { clearTimeout(timer); reject(new Error(event.message)); };
        worker.onmessage = event => { clearTimeout(timer); resolve(JSON.parse(event.data)); };
        worker.postMessage(JSON.stringify({ generation: 1, kind: 'repl', input: '2 + 2' }));
      });
      worker.terminate();
      window.scasShowReplSurface('history', '', 50, 50, 500, 300, devicePixelRatio);
      window.scasReplResult(JSON.stringify(result));
    });
    await offline.waitForSelector('#repl-history .repl-result-text', { state: 'visible' });
    assert.equal((await offline.locator('#repl-history .repl-result-text').textContent()).trim(), '4');
    await offline.waitForSelector('#repl-history .repl-typeset-retry', { state: 'visible' });
    await offline.close();
    assert.deepEqual(errors, []);
    console.log('Browser smoke passed: REPL output, online MathJax rendering, and offline text fallback are all visible.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
