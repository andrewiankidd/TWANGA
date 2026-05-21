// Capture screenshots of every top-level GUI screen for the feature
// docs.
//
// Why Playwright (and not Edge headless via `--screenshot`)? Edge
// headless doesn't reliably resolve IndexedDB operations within
// `--virtual-time-budget`, which means screens that read from the
// in-browser library (Playback, Editor) capture an empty state.
// Playwright drives a real headless Chromium with proper async-
// aware waits (`waitForFunction`, `waitForSelector`) so we can
// snapshot AFTER the library list has actually populated.
//
// Prereqs (one-time):
//   cd tools
//   npm install
//   npx playwright install chromium
//
// Run from the tools/ directory (or anywhere — paths are absolute):
//   cd tools
//   npm run screenshots
//
// Output is `docs/features/screenshots/{name}.png`. The CI step
// that mirrors docs/features into frontend/web/assets/docs picks
// the screenshots up automatically. Designed to be deterministic
// (same viewport, same theme, same wait conditions) so re-running
// produces a clean diff (or no diff at all) in git.
//
// Screens can declare multiple `shots` so a single screen produces
// several screenshots showing different states (e.g. Playback's
// library + per-tab Tab view + per-tab Highway view). When `shots`
// is omitted a single default shot is captured.

import { chromium } from 'playwright';
import { mkdir } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, '..');
const OUT_DIR = resolve(REPO_ROOT, 'docs', 'features', 'screenshots');
const BASE_URL = process.env.TWANGA_DOCS_BASE_URL || 'http://localhost:8000/app.html';

// Capture dimensions. The width is a sensible-for-docs middle
// ground — wider than mobile, narrower than a full-width desktop
// session — so the screenshot reads as an illustration of the UI
// rather than a 1:1 reproduction. The default max height clips
// sections that are taller than a thumbnail-worth of content;
// individual shots can override via `maxHeight`.
const CAPTURE_WIDTH = 1100;
const DEFAULT_MAX_HEIGHT = 800;

// One entry per screen. `waitFor` runs once after navigation; per-shot
// `setup` runs before each capture so multi-shot screens can drive UI
// state in between. State carries across shots within a screen since
// they share the same page; navigating to the next screen resets it.
const SCREENS = [
    {
        id: 'tuner',
        // Tuner sets a default tuning on mount; no async wait needed.
    },
    {
        id: 'recorder',
        // Recorder lands on the settings + empty score view; the
        // tuning controller mounts synchronously after page load.
    },
    {
        id: 'playback',
        // Playback's library list is `Promise.all([manifest fetch,
        // IndexedDB read])`. Wait for at least one library row to
        // exist before screenshotting, otherwise we get an empty
        // div where the bundled examples should be.
        waitFor: '#playback-library-list .tab-library-row',
        shots: [
            {
                // 1) Library list — shorter cap than the default so
                //    the thumbnail reads as "this is the library" not
                //    "here are 30 rows of content".
                name: 'playback',
                maxHeight: 560,
            },
            {
                // 2) Per-tab view with Tab renderer + Fret labels.
                //    Loads the first bundled example (Twinkle Twinkle)
                //    so the renderer has a real score to display.
                //    Tall cap so the renderer AND the Play / Pause /
                //    Stop transport row both fit; the per-tab view is
                //    bounded by its own controls so we don't risk a
                //    runaway-list capture like the library does.
                name: 'playback-tab-fret',
                maxHeight: 1400,
                async setup(page) {
                    // Click the first row's Load button to enter the
                    // per-tab view. Library rows are <div class
                    // ="tab-library-row"> with a <button class
                    // ="primary"> Load action.
                    await page.click('#playback-library-list .tab-library-row:first-child button.primary');
                    await page.waitForSelector('#playback-tab-view:not([hidden])', { timeout: 5000 });
                    // Force the Tab view + Fret labels in case
                    // localStorage persisted a different choice from
                    // a prior run. The handlers replay state across
                    // remounts so this is enough.
                    await page.selectOption('#playback-renderer-select', 'twanga.tab');
                    await page.selectOption('#playback-cell-label-select', 'fret');
                    // Let the renderer rebuild + the playhead settle.
                    await page.waitForTimeout(400);
                },
            },
            {
                // 3) Per-tab view with Highway renderer + Note labels.
                //    Continues from shot 2 (still loaded) so we only
                //    toggle the two view selects.
                name: 'playback-highway-note',
                maxHeight: 1400,
                async setup(page) {
                    await page.selectOption('#playback-renderer-select', 'twanga.highway');
                    await page.selectOption('#playback-cell-label-select', 'note');
                    await page.waitForTimeout(400);
                },
            },
        ],
    },
    {
        id: 'patterns',
        // Patterns also fetches a manifest, but the render path is
        // synchronous-after-fetch and finishes quickly. Wait for the
        // first group header to confirm content is in place.
        waitFor: '#patterns-library .patterns-group',
    },
    {
        id: 'editor',
        // Editor shares Playback's library backend (same IndexedDB +
        // manifest path), so the same wait applies. The editor's
        // library uses the `editor-library-list` id.
        waitFor: '#editor-library-list .tab-library-row',
    },
    {
        id: 'tunings',
        // Tunings renders from the synchronous built-in registry +
        // a localStorage read; no fetch chain to wait on.
    },
];

async function captureShot(page, screen, shot) {
    if (shot.setup) await shot.setup(page);
    // Brief settle for any post-setup layout work.
    await page.waitForTimeout(200);

    const section = page.locator(`#screen-${screen.id}`);
    const box = await section.boundingBox();
    if (!box) {
        console.warn(`  ⚠ no bounding box for #screen-${screen.id}, skipping shot ${shot.name}`);
        return;
    }
    const maxHeight = shot.maxHeight ?? DEFAULT_MAX_HEIGHT;
    const clipHeight = Math.min(box.height, maxHeight);
    // Playwright's `clip` is bounded by the current viewport — if the
    // clip's bottom edge would exceed `viewport.height`, the capture
    // is silently truncated. Resize the viewport when the shot needs
    // more vertical room (e.g. Playback per-tab view at ~900 px). The
    // section's bounding box is computed in document coords so the
    // resize doesn't change its dimensions.
    const requiredHeight = Math.ceil(box.y + clipHeight) + 20;
    const viewport = page.viewportSize();
    if (viewport && viewport.height < requiredHeight) {
        await page.setViewportSize({ width: viewport.width, height: requiredHeight });
        await page.waitForTimeout(150);
    }
    const outPath = resolve(OUT_DIR, `${shot.name}.png`);
    await page.screenshot({
        path: outPath,
        clip: { x: box.x, y: box.y, width: box.width, height: clipHeight },
    });
    console.log(`  ✓ ${outPath}  (${Math.round(box.width)}×${Math.round(clipHeight)})`);
}

async function main() {
    await mkdir(OUT_DIR, { recursive: true });

    const browser = await chromium.launch();
    const context = await browser.newContext({
        // Initial viewport tall enough to fit any single-screen
        // section without needing a resize mid-run. `captureShot`
        // bumps further if a specific shot needs more room — the
        // viewport must be at least as tall as the clip's bottom
        // edge or Playwright silently truncates the screenshot.
        viewport: { width: CAPTURE_WIDTH, height: 1600 },
        deviceScaleFactor: 1,
        colorScheme: 'dark',
    });
    const page = await context.newPage();

    let totalShots = 0;
    for (const screen of SCREENS) {
        const url = `${BASE_URL}#${screen.id}`;
        console.log(`→ ${screen.id}  (${url})`);
        await page.goto(url, { waitUntil: 'networkidle' });
        await page.waitForSelector(`#screen-${screen.id}:not([hidden])`, {
            state: 'visible',
            timeout: 5000,
        });
        if (screen.waitFor) {
            await page.waitForSelector(screen.waitFor, { timeout: 10000 })
                .catch((e) => console.warn(`  ⚠ wait '${screen.waitFor}' timed out: ${e.message}`));
        }

        const shots = screen.shots ?? [{ name: screen.id }];
        for (const shot of shots) {
            await captureShot(page, screen, shot);
            totalShots++;
        }
    }

    await browser.close();
    console.log(`\nDone — ${totalShots} screenshots written to ${OUT_DIR}`);
}

main().catch((err) => {
    console.error('capture failed:', err);
    process.exit(1);
});
