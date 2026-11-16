import { test, expect } from '@playwright/test';

const canvas = page => page.locator('#dockspace-canvas');
const state = page => canvas(page).evaluate(element => JSON.parse(element.dataset.dockspaceState));
const pane = (snapshot, item) => snapshot.panes.find(pane => pane.item === item);
const topology = snapshot => snapshot.panes.map(({ contentRect, ...pane }) => pane);

async function open(page) {
  await page.goto('demo/egui/?test=1');
  await expect(page.locator('#demo-status')).toHaveAttribute('data-state', 'ready');
  await expect.poll(async () => (await state(page)).tabs.length).toBe(8);
}

async function point(page, rect) {
  return canvas(page).evaluate((element, rect) => {
    const box = element.getBoundingClientRect();
    // AccessKit bounds use egui points; its root carries the physical-pixel transform.
    const scale = JSON.parse(element.dataset.dockspaceState).pixelsPerPoint;
    return {
      x: box.x + (rect[0] + rect[2]) / 2 * scale * box.width / element.width,
      y: box.y + (rect[1] + rect[3]) / 2 * scale * box.height / element.height,
    };
  }, rect);
}

async function clickControl(page, label) {
  const control = (await state(page)).controls
    .filter(node => node.label === label && node.role !== 'Label')
    .sort((a, b) => a.rect[1] - b.rect[1])[0];
  expect(control, `presented control ${label}`).toBeTruthy();
  const position = await point(page, control.rect);
  await page.mouse.click(position.x, position.y);
}

async function presentedPointer(page, position, down) {
  await expect.poll(() => canvas(page).evaluate((element, { position, down }) => {
    const snapshot = JSON.parse(element.dataset.dockspaceState);
    const box = element.getBoundingClientRect();
    const pointer = snapshot.input.pointer;
    return pointer !== null && snapshot.input.primaryDown === down
      && Math.abs(box.x + pointer[0] * snapshot.pixelsPerPoint * box.width / element.width - position.x) < 1
      && Math.abs(box.y + pointer[1] * snapshot.pixelsPerPoint * box.height / element.height - position.y) < 1;
  }, { position, down })).toBe(true);
}

async function startDrag(page, label, destination) {
  const tab = (await state(page)).tabs.find(tab => tab.label === label);
  const source = await point(page, tab.rect);
  const before = (await state(page)).frames;
  await page.mouse.move(source.x, source.y);
  await expect.poll(async () => (await state(page)).frames).toBeGreaterThan(before);
  await page.mouse.down();
  await presentedPointer(page, source, true);
  await page.mouse.move(destination.x, destination.y, { steps: 12 });
  await presentedPointer(page, destination, true);
  await expect.poll(async () => (await state(page)).activeGesture).toBe(true);
}

async function mergeLeft(page, label, expected) {
  const snapshot = await state(page);
  const destination = await point(page, pane(snapshot, 3).contentRect);
  await startDrag(page, label, destination);
  await expect.poll(async () => (await state(page)).centerGuides.length).toBeGreaterThan(0);
  const guides = (await state(page)).centerGuides;
  const guide = guides.sort((a, b) => a.rect[0] - b.rect[0])[0];
  const target = await point(page, guide.rect);
  await page.mouse.move(target.x, target.y, { steps: 6 });
  await presentedPointer(page, target, true);
  await page.mouse.up();
  await presentedPointer(page, target, false);
  await expect.poll(async () => pane(await state(page), 3).group).toEqual(expected);
  await expect.poll(async () => (await state(page)).activeGesture).toBe(false);
  await expect.poll(async () => {
    const snapshot = await state(page);
    const tab = snapshot.tabs.find(tab => tab.label === label);
    const bounds = pane(snapshot, 3).contentRect;
    return tab && tab.rect[0] >= bounds[0] && tab.rect[2] <= bounds[2];
  }).toBe(true);
}

test('homepage and demo resolve under a project path, including narrow screens', async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('./');
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Docking layouts');
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.getByRole('link', { name: 'Try the egui demo' }).click();
  await expect(page.locator('#demo-status')).toHaveAttribute('data-state', 'ready');
  await expect(canvas(page)).not.toHaveAttribute('data-dockspace-state');
  await page.getByRole('link', { name: 'Back to Dockspace home' }).click();
  await expect(page).toHaveURL(/\/site\/$/);
  expect(errors).toEqual([]);
});

test('real input merges tabs, retires a floating group, resizes, edits and scrolls', async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  // Exercise fallback even on a machine whose browser exposes a WebGPU adapter.
  await page.addInitScript(() => Object.defineProperty(navigator, 'gpu', { value: undefined }));
  await open(page);
  const original = await state(page);
  await mergeLeft(page, 'Preview', [3, 2]);
  await mergeLeft(page, 'Inspector', [3, 2, 4]);
  await expect.poll(async () => (await state(page)).panes.every(pane => pane.root === 1)).toBe(true);
  expect((await state(page)).panes.map(pane => pane.item)).toEqual(original.panes.map(pane => pane.item));
  expect((await state(page)).editorText).toBe(original.editorText);

  const splitter = (await state(page)).splitters.find(node =>
    node.label === 'Resize panes' && node.rect[2] - node.rect[0] < 20 && node.rect[1] < 100);
  const divider = await point(page, splitter.rect);
  await page.mouse.move(divider.x, divider.y);
  await page.mouse.down();
  await page.mouse.move(divider.x + 65, divider.y, { steps: 10 });
  await presentedPointer(page, { x: divider.x + 65, y: divider.y }, true);
  await page.mouse.up();
  await presentedPointer(page, { x: divider.x + 65, y: divider.y }, false);
  await expect.poll(async () => (await state(page)).splitters.some(node =>
    node.label === 'Resize panes' && node.rect[0] > splitter.rect[0] + 40 && node.rect[1] < 100)).toBe(true);

  let snapshot = await state(page);
  const editor = await point(page, snapshot.editorRect);
  await page.mouse.click(editor.x, editor.y);
  await presentedPointer(page, editor, false);
  await expect.poll(async () => (await state(page)).editorFocused).toBe(true);
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('// browserinput');
  await expect.poll(async () => (await state(page)).editorText).toContain('// browserinput');
  snapshot = await state(page);
  const console = await point(page, snapshot.consoleRect);
  await page.mouse.move(console.x, console.y);
  await page.mouse.wheel(0, 450);
  await expect.poll(async () => (await state(page)).consoleOffset).toBeGreaterThan(0);
  expect((await state(page)).version).toBe(snapshot.version);
  const edited = (await state(page)).editorText;
  await clickControl(page, 'Close Editor');
  await expect.poll(async () => pane(await state(page), 1)).toBeUndefined();
  await clickControl(page, 'Restore layout');
  await expect.poll(async () => (await state(page)).panes.length).toBe(8);
  expect((await state(page)).editorText).toBe(edited);
  expect(errors).toEqual([]);
});

test('blur cancels a held drag, and fresh input can float and dock again', async ({ page }) => {
  await open(page);
  const destination = await point(page, [100, 300, 200, 400]);
  await startDrag(page, 'Preview', destination);
  await page.locator('#demo-fullscreen').focus();
  await expect.poll(async () => (await state(page)).activeGesture).toBe(false);
  const cancelled = await state(page);
  await page.mouse.move(destination.x + 50, destination.y + 50);
  await page.mouse.up();
  await expect.poll(async () => (await state(page)).frames).toBeGreaterThan(cancelled.frames);
  expect(topology(await state(page))).toEqual(topology(cancelled));
  await mergeLeft(page, 'Preview', [3, 2]);

  const floatingTarget = await point(page, [650, 320, 750, 420]);
  await page.keyboard.down('Shift');
  await startDrag(page, 'Preview', floatingTarget);
  await page.mouse.up();
  await presentedPointer(page, floatingTarget, false);
  await page.keyboard.up('Shift');
  await expect.poll(async () => pane(await state(page), 2).root).not.toBe(1);
  await mergeLeft(page, 'Preview', [3, 2]);
});

test('a menu-floated layout moves over blank canvas and docks through an explicit guide', async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await open(page);
  const original = topology(await state(page));
  await clickControl(page, 'Layout actions');
  await expect.poll(async () => (await state(page)).controls.some(node => node.label === 'Float Layout')).toBe(true);
  await clickControl(page, 'Float Layout');
  const title = snapshot => snapshot.controls.find(node => node.role === 'TitleBar' && node.label === 'Outline');
  await expect.poll(async () => Boolean(title(await state(page)))).toBe(true);

  const blank = await point(page, [150, 500, 150, 500]);
  const dragWindow = async () => {
    const source = await point(page, title(await state(page)).rect);
    await page.mouse.move(source.x, source.y);
    await presentedPointer(page, source, false);
    await page.mouse.down();
    await presentedPointer(page, source, true);
    await page.mouse.move(blank.x, blank.y, { steps: 12 });
    await presentedPointer(page, blank, true);
    await expect.poll(async () => (await state(page)).activeGesture).toBe(true);
    await expect.poll(async () => (await state(page)).centerGuides.length).toBe(1);
  };

  await dragWindow();
  await page.mouse.up();
  await presentedPointer(page, blank, false);
  await expect.poll(async () => (await state(page)).activeGesture).toBe(false);
  expect(title(await state(page)), 'blank canvas keeps the complete layout floating').toBeTruthy();
  expect(topology(await state(page))).toEqual(original);

  await dragWindow();
  const guide = await point(page, (await state(page)).centerGuides[0].rect);
  await page.mouse.move(guide.x, guide.y, { steps: 12 });
  await presentedPointer(page, guide, true);
  await page.mouse.up();
  await presentedPointer(page, guide, false);
  await expect.poll(async () => (await state(page)).activeGesture).toBe(false);
  await expect.poll(async () => Boolean(title(await state(page)))).toBe(false);
  expect(topology(await state(page))).toEqual(original);
  expect(errors).toEqual([]);
});

test('motion produces intermediate pane geometry, accepts tuning and settles to idle', async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    const element = document.querySelector('#dockspace-canvas');
    window.presentedSamples = [];
    new MutationObserver(() => window.presentedSamples.push(JSON.parse(element.dataset.dockspaceState)))
      .observe(element, { attributes: true, attributeFilter: ['data-dockspace-state'] });
  });
  await clickControl(page, 'Close Outline');
  await expect.poll(async () => (await state(page)).panes.length).toBe(7);
  await expect.poll(async () => pane(await state(page), 1).contentRect[0]).toBeLessThan(30);
  const positions = await page.evaluate(() => window.presentedSamples
    .map(sample => sample.panes.find(pane => pane.item === 1)?.contentRect?.[0])
    .filter(value => value > 30 && value < 350));
  expect(positions.length).toBeGreaterThan(0);

  await clickControl(page, 'Spring');
  await expect.poll(async () => (await state(page)).motion.settings).toContain('CriticalSpring');
  await clickControl(page, 'Tween');
  await expect.poll(async () => (await state(page)).motion.settings).toContain('layout: Tween');
  const slider = (await state(page)).controls.find(node => node.role === 'Slider');
  const thumb = await point(page, [
    slider.rect[0] + (slider.rect[2] - slider.rect[0]) * 2 / 3, slider.rect[1],
    slider.rect[0] + (slider.rect[2] - slider.rect[0]) * 2 / 3, slider.rect[3],
  ]);
  const maximum = await point(page, [slider.rect[2] - 1, slider.rect[1], slider.rect[2] - 1, slider.rect[3]]);
  await page.mouse.move(thumb.x, thumb.y);
  await page.mouse.down();
  await page.mouse.move(maximum.x, maximum.y, { steps: 6 });
  await page.mouse.up();
  await expect.poll(async () => (await state(page)).motion.settings).toContain('duration: 300ms');
  const combo = (await state(page)).controls.find(node => node.role === 'ComboBox' && node.text === 'EaseInOut');
  const curveMenu = await point(page, combo.rect);
  await page.mouse.click(curveMenu.x, curveMenu.y);
  await expect.poll(async () => (await state(page)).controls.some(node => node.label === 'Ease out')).toBe(true);
  await clickControl(page, 'Ease out');
  await expect.poll(async () => (await state(page)).motion.settings).toContain('curve: EaseOut');
  await clickControl(page, 'Motion');
  await expect.poll(async () => (await state(page)).motion.settings).toContain('mode: Off');
  await clickControl(page, 'Close Console');
  await expect.poll(async () => pane(await state(page), 5)).toBeUndefined();
  await page.evaluate(() => { window.presentedSamples = []; });
  await clickControl(page, 'Close History');
  await expect.poll(async () => pane(await state(page), 7).contentRect[0]).toBeLessThan(30);
  const disabledPositions = await page.evaluate(() => window.presentedSamples
    .map(sample => sample.panes.find(pane => pane.item === 7)?.contentRect?.[0])
    .filter(value => value > 30 && value < 900));
  expect(disabledPositions).toEqual([]);
  await page.mouse.move(10, 980);
  // A finite quiet interval verifies the absence of a perpetual animation loop.
  await page.waitForTimeout(700);
  const settled = (await state(page)).frames;
  await page.waitForTimeout(300);
  expect((await state(page)).frames).toBe(settled);
});

test('tab presets switch live, follow themes and retain custom tabs and docking', async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await open(page);
  const original = await state(page);
  for (const [label, preset] of [['ImGui', 'ImGui'], ['egui_tiles', 'EguiTiles'], ['Dockspace', 'Dockspace']]) {
    const combo = (await state(page)).controls.find(node => node.role === 'ComboBox'
      && ['Dockspace', 'ImGui', 'egui_tiles'].includes(node.text));
    expect(combo, 'presented tab-style selector').toBeTruthy();
    const position = await point(page, combo.rect);
    await page.mouse.click(position.x, position.y);
    await expect.poll(async () => (await state(page)).controls.some(node => node.label === label)).toBe(true);
    await clickControl(page, label);
    await expect.poll(async () => (await state(page)).tabPreset).toBe(preset);
    await clickControl(page, 'Dark');
    await expect.poll(async () => (await state(page)).darkMode).toBe(true);
    await clickControl(page, 'Light');
    await expect.poll(async () => (await state(page)).darkMode).toBe(false);
    await expect.poll(async () => (await state(page)).tabs.length).toBe(8);
    expect(topology(await state(page))).toEqual(topology(original));
  }
  const beforeCustom = (await state(page)).tabs.find(tab => tab.label === 'Preview').rect;
  await clickControl(page, 'Custom tabs');
  await expect.poll(async () => {
    const rect = (await state(page)).tabs.find(tab => tab.label === 'Preview').rect;
    return rect[2] - rect[0];
  }).toBeGreaterThan(beforeCustom[2] - beforeCustom[0] + 10);
  await mergeLeft(page, 'Preview', [3, 2]);
  expect((await state(page)).editorText).toBe(original.editorText);
  expect(errors).toEqual([]);
});

test('reduced motion follows system changes and preserves a manual toolbar choice', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await open(page);
  expect((await state(page)).motion.preference).toBe('Reduced');
  await clickControl(page, 'Reduce motion');
  await expect.poll(async () => (await state(page)).motion.preference).toBe('Unknown');
  await page.setViewportSize({ width: 1380, height: 920 });
  await expect.poll(async () => (await state(page)).pixelsPerPoint).toBe(1);
  expect((await state(page)).motion.preference).toBe('Unknown');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect.poll(async () => (await state(page)).motion.preference).toBe('Full');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect.poll(async () => (await state(page)).motion.preference).toBe('Reduced');
});

test('initialization failures leave a readable error', async ({ page }) => {
  await page.route('**/egui_web_demo_bg.wasm', route => route.abort('failed'));
  await page.goto('demo/egui/');
  await expect(page.locator('#demo-status')).toHaveAttribute('data-state', 'error');
  await expect(page.locator('#demo-status')).toContainText('could not start');
});

test('native device scale two uses the real backing size for docking', async ({ playwright, baseURL }) => {
  const browser = await playwright.chromium.launch({
    channel: 'chromium',
    args: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader', '--force-device-scale-factor=2', '--window-size=1440,1000'],
  });
  try {
    // Do not override DPR through CDP: it can disagree with devicePixelContentBoxSize.
    const page = await browser.newPage({ viewport: null, baseURL });
    await open(page);
    const backing = await canvas(page).evaluate(element => ({
      width: element.width, logicalWidth: element.clientWidth, dpr: devicePixelRatio,
    }));
    expect(backing.dpr).toBe(2);
    expect(backing.width).toBe(backing.logicalWidth * 2);
    expect((await state(page)).pixelsPerPoint).toBe(2);
    await mergeLeft(page, 'Preview', [3, 2]);
    await expect(page.locator('#demo-status')).toHaveAttribute('data-state', 'ready');
  } finally {
    await browser.close();
  }
});
