// Tests for the wait-mode reading-match helpers.
//
// Run with:  node --test frontend/web/lib/wait-match.test.js
//
// Pure stdlib — uses Node 22+'s built-in `node:test` (the CI workflow
// runs `node --test "frontend/web/**/*.test.js"` so any new test
// file gets picked up automatically). No installed deps; the helpers
// are pure functions so no jsdom / WASM / playback-loop setup
// is needed.

import { test } from 'node:test';
import assert from 'node:assert/strict';

import { waitReadingAdvances, findFirstAdvancingReading } from './wait-match.js';

/// Shape matches what `WebTuner.take_readings()` returns. Only the
/// fields the helpers actually consume are populated — the others
/// (label / target_hz / cents) are left out so it's obvious what
/// the contract reads.
function reading({ detected_hz = 440, from_onset_window = true } = {}) {
    return { detected_hz, from_onset_window };
}

// ───────────────────────── waitReadingAdvances ─────────────────────────

test('waitReadingAdvances: onset + match → true', () => {
    const r = reading({ detected_hz: 440, from_onset_window: true });
    assert.equal(waitReadingAdvances(r, (hz) => hz === 440), true);
});

test('waitReadingAdvances: onset + no-match → false', () => {
    const r = reading({ detected_hz: 440, from_onset_window: true });
    assert.equal(waitReadingAdvances(r, () => false), false);
});

test('waitReadingAdvances: no-onset + match → false (sustained-tail guard)', () => {
    // The whole point of the gate: a reading whose pitch matches
    // but came from a non-onset window (i.e. the previous note's
    // sustained tail) MUST NOT advance the cursor.
    const r = reading({ detected_hz: 440, from_onset_window: false });
    assert.equal(waitReadingAdvances(r, () => true), false);
});

test('waitReadingAdvances: no-onset + no-match → false', () => {
    const r = reading({ detected_hz: 440, from_onset_window: false });
    assert.equal(waitReadingAdvances(r, () => false), false);
});

test('waitReadingAdvances: null reading → false (defensive)', () => {
    assert.equal(waitReadingAdvances(null, () => true), false);
    assert.equal(waitReadingAdvances(undefined, () => true), false);
});

test('waitReadingAdvances: non-function matchFn → false (defensive)', () => {
    const r = reading();
    assert.equal(waitReadingAdvances(r, null), false);
    assert.equal(waitReadingAdvances(r, 'not-a-function'), false);
});

test('waitReadingAdvances: missing from_onset_window field is treated as false', () => {
    // Older WebTuner builds (pre-Ship-1) might return readings
    // without the from_onset_window field. The gate should default
    // to "don't advance" so the CLI / web behaviour matches the
    // post-fix semantics (no false-positive auto-advances).
    const legacy = { detected_hz: 440 };
    assert.equal(waitReadingAdvances(legacy, () => true), false);
});

test('waitReadingAdvances: matchFn receives detected_hz', () => {
    const r = reading({ detected_hz: 196.0 });
    let received = null;
    waitReadingAdvances(r, (hz) => {
        received = hz;
        return false;
    });
    assert.equal(received, 196.0);
});

// ───────────────────────── findFirstAdvancingReading ─────────────────────────

test('findFirstAdvancingReading: returns first advancing reading', () => {
    const readings = [
        reading({ detected_hz: 100, from_onset_window: false }),
        reading({ detected_hz: 200, from_onset_window: true }),
        reading({ detected_hz: 300, from_onset_window: true }),
    ];
    const hit = findFirstAdvancingReading(readings, (hz) => hz === 200 || hz === 300);
    assert.equal(hit?.detected_hz, 200, 'should return the FIRST matching reading');
});

test('findFirstAdvancingReading: skips non-onset readings even when they match', () => {
    // A sustained-tail reading at the expected pitch must not be
    // picked even if it comes before a true-onset reading at a
    // different pitch.
    const readings = [
        reading({ detected_hz: 440, from_onset_window: false }),
        reading({ detected_hz: 220, from_onset_window: true }),
    ];
    const hit = findFirstAdvancingReading(readings, () => true);
    assert.equal(hit?.detected_hz, 220);
});

test('findFirstAdvancingReading: returns null when nothing advances', () => {
    const readings = [
        reading({ from_onset_window: true }),
        reading({ from_onset_window: true }),
    ];
    assert.equal(findFirstAdvancingReading(readings, () => false), null);
});

test('findFirstAdvancingReading: returns null on empty / missing input', () => {
    assert.equal(findFirstAdvancingReading([], () => true), null);
    assert.equal(findFirstAdvancingReading(null, () => true), null);
    assert.equal(findFirstAdvancingReading(undefined, () => true), null);
});

test('findFirstAdvancingReading: stops iterating at the first match', () => {
    // Side-effect counter on matchFn proves we don't process
    // remaining readings after a hit. Wait-mode advances exactly
    // once per pluck — extra iterations would be wasted work and
    // could mask a bug where two readings both "advance" but only
    // one fires the actual cursor move.
    let calls = 0;
    const readings = [
        reading({ detected_hz: 100 }),
        reading({ detected_hz: 200 }),
        reading({ detected_hz: 300 }),
    ];
    findFirstAdvancingReading(readings, (hz) => {
        calls += 1;
        return hz === 100;
    });
    assert.equal(calls, 1, 'matchFn should have been called only for the first reading');
});
