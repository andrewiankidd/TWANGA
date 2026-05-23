// Shared input-device picker. Same factory shape as makeMicMeter /
// makeTuningController — each mic-consuming screen owns its own
// instance, passes in the DOM id of its <select> + a localStorage
// key, and reads `getSelectedId()` when it's about to open the mic.
//
// Browsers gate device LABELS behind an existing mic permission
// grant — `enumerateDevices()` on first load returns devices with
// empty `label` fields. We populate the select with whatever's
// available now, then re-enumerate after the first successful
// `getUserMedia()` to fill the labels in. Tauri's trusted webview
// returns labels immediately (no permission gate) so the same code
// path "just works" there.
//
// Required markup (matches `.device-picker` CSS in app.html):
//
//   <div class="device-picker">
//     <label for="..." class="device-picker-label">Input</label>
//     <select id="..." class="device-picker-select"></select>
//   </div>

const DEFAULT_OPTION_VALUE = '__default__';

export function makeDevicePicker({ selectId, storageKey, onChange }) {
    const select = document.getElementById(selectId);
    if (!select) {
        throw new Error(`makeDevicePicker: missing element ${selectId}`);
    }

    let selectedId = loadSelectedId(storageKey);

    async function enumerate() {
        try {
            const devices = await navigator.mediaDevices.enumerateDevices();
            return devices.filter((d) => d.kind === 'audioinput');
        } catch (e) {
            console.warn('enumerateDevices failed', e);
            return [];
        }
    }

    function populate(devices) {
        const prevSelected = selectedId;
        // Snapshot the saved choice, blow away the options, rebuild.
        select.innerHTML = '';

        const defaultOpt = document.createElement('option');
        defaultOpt.value = DEFAULT_OPTION_VALUE;
        defaultOpt.textContent = 'Default (OS-selected)';
        select.appendChild(defaultOpt);

        let foundPrev = prevSelected === null;
        for (const d of devices) {
            const opt = document.createElement('option');
            opt.value = d.deviceId;
            // Browsers without permission return empty `label` —
            // show a synthetic name so the list isn't a wall of
            // blank rows.
            opt.textContent = d.label?.trim()
                || `Input ${d.deviceId.slice(0, 6) || '(unnamed)'}`;
            if (d.deviceId === prevSelected) {
                opt.selected = true;
                foundPrev = true;
            }
            select.appendChild(opt);
        }

        if (!foundPrev) {
            // Saved device id isn't in the current list (unplugged,
            // permission revoked, etc). Fall back to default + clear
            // storage so we don't keep trying a phantom.
            select.value = DEFAULT_OPTION_VALUE;
            selectedId = null;
            saveSelectedId(storageKey, null);
        }
    }

    function getSelectedId() {
        return selectedId;
    }

    /// Human-readable label of the currently-selected input. Used
    /// by latency calibration as the per-device key — browsers
    /// expose stable labels (once permission is granted) that
    /// survive across sessions even when `deviceId` rotates.
    /// Falls back to a synthetic name when no label is available
    /// (pre-permission first-load case).
    function getSelectedLabel() {
        const opt = select.selectedOptions[0];
        return opt ? opt.textContent : 'Default (OS-selected)';
    }

    async function refresh() {
        const devices = await enumerate();
        populate(devices);
    }

    select.addEventListener('change', () => {
        const v = select.value;
        selectedId = v === DEFAULT_OPTION_VALUE ? null : v;
        saveSelectedId(storageKey, selectedId);
        if (typeof onChange === 'function') {
            onChange(selectedId);
        }
    });

    // Hot-plug — refresh the list when the user plugs / unplugs a
    // USB mic mid-session. devicechange is a MediaDevices event.
    if (navigator.mediaDevices?.addEventListener) {
        navigator.mediaDevices.addEventListener('devicechange', () => {
            refresh().catch((e) => console.warn('refresh on devicechange failed', e));
        });
    }

    async function start() {
        await refresh();
    }

    return { start, refresh, getSelectedId, getSelectedLabel };
}

function loadSelectedId(storageKey) {
    try {
        const raw = localStorage.getItem(storageKey);
        return raw && raw !== DEFAULT_OPTION_VALUE ? raw : null;
    } catch (e) {
        return null;
    }
}

function saveSelectedId(storageKey, deviceId) {
    try {
        if (deviceId === null) {
            localStorage.removeItem(storageKey);
        } else {
            localStorage.setItem(storageKey, deviceId);
        }
    } catch (e) {
        console.warn('saveSelectedId failed', e);
    }
}
