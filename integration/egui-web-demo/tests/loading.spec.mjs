import { test, expect } from '@playwright/test';

const status = page => page.locator('#demo-status');
const loading = page => page.locator('#demo-loading');
const progress = page => page.getByRole('progressbar', { name: 'Wasm download progress' });

test('a pending download shows its stage and offers a retry when it takes longer', async ({ page }) => {
  let resume;
  const pending = new Promise(resolve => { resume = resolve; });
  await page.route('**/egui_web_demo_bg.wasm', async route => {
    await pending;
    await route.continue();
  });
  await page.clock.install();
  try {
    await page.goto('demo/egui/', { waitUntil: 'domcontentloaded' });
    await expect(loading(page)).toHaveAttribute('data-phase', 'download');
    await expect(progress(page)).toHaveAttribute('value', '0');
    await expect(status(page)).toContainText('Downloading Wasm');
    await expect(page.getByRole('button', { name: 'Retry loading' })).toBeHidden();
    await page.clock.fastForward(15_000);
    await expect(page.locator('#demo-loading-hint')).toContainText('taking longer');
    await expect(page.getByRole('button', { name: 'Retry loading' })).toBeVisible();
  } finally {
    resume();
  }
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  await expect(loading(page)).toBeHidden();
  await expect(page.locator('.demo-stage')).toHaveAttribute('aria-busy', 'false');
});

test('streamed download progress uses decoded Wasm bytes rather than compressed content length', async ({ page }) => {
  await page.addInitScript(() => {
    const fetch = window.fetch.bind(window);
    window.fetch = async (...args) => {
      const response = await fetch(...args);
      if (!response.url.endsWith('/egui_web_demo_bg.wasm')) return response;
      const bytes = new Uint8Array(await response.arrayBuffer());
      let offset = 0;
      const body = new ReadableStream({
        start(controller) {
          window.nextWasmChunk = () => {
            const end = Math.min(bytes.length, offset + Math.ceil(bytes.length / 4));
            controller.enqueue(bytes.subarray(offset, end));
            offset = end;
            if (offset === bytes.length) controller.close();
          };
        },
      });
      return new Response(body, {
        headers: {
          'content-type': 'application/wasm',
          'content-encoding': 'br',
          'content-length': String(Math.floor(bytes.length / 5)),
        },
      });
    };
  });
  await page.goto('demo/egui/', { waitUntil: 'domcontentloaded' });
  await expect.poll(() => page.evaluate(() => typeof window.nextWasmChunk)).toBe('function');
  await page.evaluate(() => window.nextWasmChunk());
  await expect(progress(page)).toHaveAttribute('value', '25');
  await expect(page.locator('#demo-loading-percent')).toHaveText('25%');
  await expect(page.locator('#demo-loading-detail')).toContainText('MiB');
  await page.evaluate(() => window.nextWasmChunk());
  await expect(progress(page)).toHaveAttribute('value', '50');
  await page.evaluate(() => window.nextWasmChunk());
  await expect(progress(page)).toHaveAttribute('value', '75');
  await expect(status(page)).toHaveAttribute('data-state', 'loading');
  await page.evaluate(() => window.nextWasmChunk());
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  await expect(loading(page)).toBeHidden();
});

test('a complete download remains visibly compiling until compilation finishes', async ({ page }) => {
  await page.addInitScript(() => {
    const compile = WebAssembly.compileStreaming.bind(WebAssembly);
    WebAssembly.compileStreaming = async (...args) => {
      const module = await compile(...args);
      await new Promise(resolve => { window.finishWasmCompilation = resolve; });
      return module;
    };
  });
  await page.goto('demo/egui/', { waitUntil: 'domcontentloaded' });
  await expect(loading(page)).toHaveAttribute('data-phase', 'compile');
  await expect(progress(page)).toHaveAttribute('value', '100');
  await expect(status(page)).toContainText('Compiling Wasm');
  await expect(loading(page)).toBeVisible();
  await expect.poll(() => page.evaluate(() => typeof window.finishWasmCompilation)).toBe('function');
  await page.evaluate(() => window.finishWasmCompilation());
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  await expect(loading(page)).toBeHidden();
});

test('graphics startup stays visible until the first frame is presented', async ({ page }) => {
  await page.clock.install();
  await page.addInitScript(() => {
    const requestFrame = window.requestAnimationFrame.bind(window);
    const frames = [];
    window.requestAnimationFrame = callback => {
      frames.push(callback);
      return frames.length;
    };
    window.startDemoFrames = () => {
      window.requestAnimationFrame = requestFrame;
      for (const callback of frames) requestFrame(callback);
    };
  });
  await page.goto('demo/egui/?test=1', { waitUntil: 'domcontentloaded' });
  await expect(loading(page)).toHaveAttribute('data-phase', 'graphics');
  await expect(progress(page)).toHaveAttribute('value', '100');
  await expect(page.locator('#demo-loading-phase')).toContainText('graphics');
  await expect(loading(page)).toBeVisible();
  const webgl = page.getByRole('link', { name: 'Try WebGL2' });
  await expect(webgl).toBeHidden();
  await page.clock.fastForward(15_000);
  await expect(webgl).toBeVisible();
  await expect(webgl).toHaveAttribute('href', /\?test=1&renderer=webgl$/);
  await page.evaluate(() => window.startDemoFrames());
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  await expect(loading(page)).toBeHidden();
});

test('a stalled WebGPU request falls back to WebGL2 and ignores late completion', async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'gpu', {
      value: {
        requestAdapter: () => new Promise(resolve => { window.finishWebGpuRequest = resolve; }),
      },
    });
    window.graphicsContexts = [];
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (type, ...args) {
      window.graphicsContexts.push(type);
      return getContext.call(this, type, ...args);
    };
  });
  await page.clock.install();
  await page.goto('demo/egui/', { waitUntil: 'domcontentloaded' });
  await expect.poll(() => page.evaluate(() => typeof window.finishWebGpuRequest)).toBe('function');
  await page.clock.fastForward(11_000);
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  await expect(loading(page)).toBeHidden();
  const contexts = await page.evaluate(() => window.graphicsContexts);
  expect(contexts).toContain('webgl2');
  expect(contexts).not.toContain('webgpu');
  await page.evaluate(() => window.finishWebGpuRequest(null));
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  expect(errors).toEqual([]);
});

test('explicit WebGL2 startup bypasses WebGPU requests', async ({ page }) => {
  await page.addInitScript(() => {
    window.webGpuRequests = 0;
    Object.defineProperty(navigator, 'gpu', {
      value: {
        requestAdapter: () => {
          window.webGpuRequests += 1;
          return new Promise(() => {});
        },
      },
    });
  });
  await page.goto('demo/egui/?renderer=webgl', { waitUntil: 'domcontentloaded' });
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  expect(await page.evaluate(() => window.webGpuRequests)).toBe(0);
});

test('unavailable graphics backends leave an error and recovery controls', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'gpu', {
      value: { requestAdapter: async () => null },
    });
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (type, ...args) {
      return type === 'webgl2' ? null : getContext.call(this, type, ...args);
    };
  });
  await page.goto('demo/egui/');
  await expect(status(page)).toHaveAttribute('data-state', 'error');
  await expect(loading(page)).toHaveAttribute('data-phase', 'error');
  await expect(page.locator('#demo-loading-detail')).toContainText('The demo stopped:');
  await expect(page.getByRole('button', { name: 'Retry loading' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Try WebGL2' })).toBeVisible();
});

test('HTTP failures are readable and retrying starts a fresh successful load', async ({ page }) => {
  let requests = 0;
  await page.route('**/egui_web_demo_bg.wasm', route => {
    requests += 1;
    return requests === 1
      ? route.fulfill({ status: 503, contentType: 'text/plain', body: 'Unavailable' })
      : route.continue();
  });
  await page.goto('demo/egui/');
  await expect(status(page)).toHaveAttribute('data-state', 'error');
  await expect(loading(page)).toBeVisible();
  await expect(page.locator('#demo-loading-detail')).toContainText('HTTP 503');
  await page.getByRole('button', { name: 'Retry loading' }).click();
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
  await expect(loading(page)).toBeHidden();
  expect(requests).toBe(2);
});

test('unknown download sizes stay indeterminate and browsers without streaming compilation can start', async ({ page }) => {
  await page.addInitScript(() => { WebAssembly.compileStreaming = undefined; });
  await page.route('**/demo/egui/', async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/name="dockspace-wasm-bytes" content="\d+"/, 'name="dockspace-wasm-bytes" content=""');
    await route.fulfill({ response, body });
  });
  let resume;
  const pending = new Promise(resolve => { resume = resolve; });
  await page.route('**/egui_web_demo_bg.wasm', async route => {
    await pending;
    await route.continue();
  });
  try {
    await page.goto('demo/egui/', { waitUntil: 'domcontentloaded' });
    await expect(loading(page)).toHaveAttribute('data-phase', 'download');
    await expect(progress(page)).not.toHaveAttribute('value');
    await expect(page.locator('#demo-loading-percent')).toHaveText('');
  } finally {
    resume();
  }
  await expect(status(page)).toHaveAttribute('data-state', 'ready');
});

for (const resource of ['loading.js', 'pkg/egui_web_demo.js', 'pkg/egui_web_demo_bg.wasm']) {
  test(`stale ${resource} from a previous deployment cannot mix with the current runtime`, async ({ page }) => {
    let staleRequests = 0;
    await page.route(`**/demo/egui/${resource}`, route => {
      staleRequests += 1;
      return route.fulfill({ contentType: 'text/javascript', body: 'throw new Error("Stale runtime from a previous deployment");' });
    });
    await page.goto('demo/egui/');
    await expect(status(page)).toHaveAttribute('data-state', 'ready');
    expect(staleRequests).toBe(0);
  });
}

test('launcher failures also leave a readable error and retry control', async ({ page }) => {
  await page.route('**/loading*.js', route => route.abort('failed'));
  await page.goto('demo/egui/');
  await expect(status(page)).toHaveAttribute('data-state', 'error');
  await expect(loading(page)).toHaveAttribute('data-phase', 'error');
  await expect(page.locator('#demo-loading-detail')).toContainText('could not start');
  await expect(page.getByRole('button', { name: 'Retry loading' })).toBeVisible();
});
