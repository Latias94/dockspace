const status = document.getElementById('demo-status');
const stage = document.querySelector('.demo-stage');
const loading = document.getElementById('demo-loading');
const title = document.getElementById('demo-loading-title');
const phaseLabel = document.getElementById('demo-loading-phase');
const detail = document.getElementById('demo-loading-detail');
const percent = document.getElementById('demo-loading-percent');
const progress = document.getElementById('demo-loading-progress');
const hint = document.getElementById('demo-loading-hint');
const retry = document.getElementById('demo-retry');
const webgl = document.getElementById('demo-webgl');
const fullscreen = document.getElementById('demo-fullscreen');
const build = document.querySelector('meta[name="dockspace-build"]').content;
const pkg = new URL(`./pkg/${build}/`, import.meta.url);

retry.addEventListener('click', () => window.location.reload());
const webglUrl = new URL(window.location.href);
const usingWebgl = webglUrl.searchParams.get('renderer') === 'webgl';
webglUrl.searchParams.set('renderer', 'webgl');
webgl.href = webglUrl.href;

if (document.fullscreenEnabled) {
  fullscreen.hidden = false;
  fullscreen.addEventListener('click', async () => {
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else {
        await document.documentElement.requestFullscreen();
      }
      document.getElementById('dockspace-canvas')?.focus({ preventScroll: true });
    } catch (error) {
      console.error('Fullscreen could not be opened.', error);
    }
  });
  document.addEventListener('fullscreenchange', () => {
    fullscreen.textContent = document.fullscreenElement ? 'Exit fullscreen' : 'Fullscreen';
  });
}

let slowTimer;

function isSettled() {
  return status.dataset.state === 'ready' || status.dataset.state === 'error';
}

function scheduleSlowHint(phase) {
  clearTimeout(slowTimer);
  if (isSettled() || document.hidden) return;
  slowTimer = setTimeout(() => {
    if (isSettled() || document.hidden) return;
    hint.textContent = phase === 'graphics'
      ? 'Graphics startup is taking longer than usual. Try WebGL2, or check graphics acceleration in your browser settings.'
      : phase === 'download' || phase === 'bootstrap'
      ? 'This is taking longer than usual. Check your connection, or try loading again.'
      : 'This is taking longer than usual. You can keep waiting, or try loading again.';
    hint.hidden = false;
    retry.hidden = false;
    webgl.hidden = phase !== 'graphics' || usingWebgl;
  }, 15_000);
}

function onVisibilityChange() {
  if (isSettled()) return;
  if (document.hidden) {
    clearTimeout(slowTimer);
  } else if (hint.hidden) {
    scheduleSlowHint(loading.dataset.phase);
  }
}

document.addEventListener('visibilitychange', onVisibilityChange);

function describeError(error) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string' && error) return error;
  return String(error ?? 'Unknown error');
}

function failStartup(error) {
  if (isSettled()) return;
  status.textContent = `The workbench could not start: ${describeError(error)}`;
  status.dataset.state = 'error';
  observeStatus();
}

function onStartupError(event) {
  if (isSettled()) return;
  failStartup(event.error ?? event.message ?? 'Unexpected browser error');
}

function onStartupRejection(event) {
  if (isSettled()) return;
  failStartup(event.reason ?? 'Unhandled promise rejection');
}

window.addEventListener('error', onStartupError);
window.addEventListener('unhandledrejection', onStartupRejection);

function cleanupStartupListeners() {
  document.removeEventListener('visibilitychange', onVisibilityChange);
  window.removeEventListener('error', onStartupError);
  window.removeEventListener('unhandledrejection', onStartupRejection);
}

function setPhase(phase, label, description) {
  loading.dataset.phase = phase;
  phaseLabel.textContent = label;
  status.textContent = label;
  detail.textContent = description;
  hint.hidden = true;
  retry.hidden = true;
  webgl.hidden = true;
  for (const step of loading.querySelectorAll('[data-step]')) {
    if (step.dataset.step === phase) step.setAttribute('aria-current', 'step');
    else step.removeAttribute('aria-current');
  }
  scheduleSlowHint(phase);
}

function observeStatus() {
  if (status.dataset.state === 'ready') {
    clearTimeout(slowTimer);
    cleanupStartupListeners();
    loading.hidden = true;
    stage.setAttribute('aria-busy', 'false');
  } else if (status.dataset.state === 'error') {
    clearTimeout(slowTimer);
    cleanupStartupListeners();
    if (loading.dataset.phase === 'graphics' && !usingWebgl) webgl.hidden = false;
    loading.hidden = false;
    loading.dataset.phase = 'error';
    title.textContent = 'Unable to open the workbench';
    phaseLabel.textContent = 'Loading stopped';
    detail.textContent = status.textContent;
    progress.hidden = true;
    percent.hidden = true;
    hint.hidden = true;
    retry.hidden = false;
    stage.setAttribute('aria-busy', 'false');
  }
}

new MutationObserver(observeStatus).observe(status, {
  attributes: true,
  attributeFilter: ['data-state'],
  childList: true,
});

const size = bytes => `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;

async function compileWasm() {
  // Fetch delivers decoded bytes; compressed Content-Length is not their total.
  const expected = Number(document.querySelector('meta[name="dockspace-wasm-bytes"]').content);
  let received = 0;
  const updateProgress = complete => {
    if (complete || expected > 0) {
      const value = complete ? 100 : Math.min(99, Math.floor(received / expected * 100));
      progress.value = value;
      percent.textContent = `${value}%`;
    }
    detail.textContent = expected > 0 && !complete
      ? `${size(received)} / ${size(expected)} of Wasm received (uncompressed)`
      : `${size(received)} of Wasm received (uncompressed)`;
    if (complete) setPhase('compile', 'Compiling Wasm…', `${size(received)} downloaded. Preparing the app for your browser.`);
  };

  setPhase('download', 'Downloading Wasm…', 'Connecting to the demo download…');
  updateProgress(false);
  const response = await fetch(new URL('egui_web_demo_bg.wasm', pkg));
  if (!response.ok) throw new Error(`Wasm download failed (HTTP ${response.status}). Please try again.`);
  if (!response.body) {
    const bytes = await response.arrayBuffer();
    received = bytes.byteLength;
    updateProgress(true);
    return WebAssembly.compile(bytes);
  }
  const stream = response.body.pipeThrough(new TransformStream({
    transform(chunk, controller) {
      received += chunk.byteLength;
      updateProgress(false);
      controller.enqueue(chunk);
    },
    flush() { updateProgress(true); },
  }));
  const monitored = new Response(stream, { headers: { 'Content-Type': 'application/wasm' } });
  if (typeof WebAssembly.compileStreaming === 'function') {
    return WebAssembly.compileStreaming(monitored);
  }
  return WebAssembly.compile(await monitored.arrayBuffer());
}

try {
  setPhase('bootstrap', 'Preparing the workbench…', 'Loading the app launcher…');
  if (typeof WebAssembly === 'undefined') throw new Error('This browser does not support WebAssembly. Please use a current browser.');
  const [{ default: init }, module] = await Promise.all([
    import(new URL('egui_web_demo.js', pkg)),
    compileWasm(),
  ]);
  setPhase('graphics', 'Initializing graphics…', 'Wasm is ready. Starting the renderer and drawing the first frame.');
  await init({ module_or_path: module });
  observeStatus();
} catch (error) {
  status.textContent = `The workbench could not start: ${error instanceof Error ? error.message : String(error)}`;
  status.dataset.state = 'error';
  observeStatus();
  console.error(error);
}
