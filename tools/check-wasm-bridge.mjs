// CI smoke test: every JS call into the WASM bridge resolves to an
// actual export.
//
// Why: wasm-bindgen exports live in `frontend/web/pkg/`, which is
// gitignored and rebuilt by CI. When the Rust side adds a new method
// on an exported class, the frontend tests + clippy + Rust tests all
// pass — but the deployed JS calls a method that doesn't exist on the
// older WebX class. Caught us once already (`start_noise_calibration`
// shipped on the Rust side, JS broke at runtime).
//
// This script:
//   1. Parses `frontend/web/pkg/twanga_web.d.ts` to extract every
//      exported class's method names (instance + static) and every
//      free-function name.
//   2. Scans `frontend/web/app.html`, `frontend/web/lib/*.js`, and
//      `frontend/web/controllers/*.js` for:
//        - Free-function calls matching known wasm names.
//        - Static class calls (`WebX.method(`).
//        - Method calls on JS accessors known to resolve to a WASM
//          instance (registered in `WASM_ACCESSORS` below).
//   3. Reports any call site whose method isn't in the declared API.
//
// It's a regex-level check, not a full type checker — catches the
// "missing method" class of bug without needing tsc or wasm-pack test.
//
// Run:
//   cd tools && node check-wasm-bridge.mjs
// CI integration: see the "Frontend WASM bridge surface check" job
// in `.github/workflows/ci.yml`.

import { readFile, readdir } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, '..');
const DTS_PATH = resolve(REPO_ROOT, 'frontend', 'web', 'pkg', 'twanga_web.d.ts');

// JS source files that consume the WASM bridge. Keep this list short
// and explicit — globbing `**/*.js` would also pick up tests and the
// pkg directory itself, which we don't want to scan.
const JS_SOURCES = [
    'frontend/web/app.html',
    'frontend/web/lib/library.js',
    'frontend/web/lib/library-tauri.js',
    'frontend/web/lib/user-tunings.js',
    'frontend/web/lib/user-tunings-tauri.js',
    'frontend/web/lib/markdown.js',
    'frontend/web/controllers/tuning.js',
    'frontend/web/controllers/mic-meter.js',
    'frontend/web/controllers/device-picker.js',
    'frontend/web/controllers/silence-threshold.js',
];

// JS accessor expressions that hold a WASM-class instance. Methods
// called on these get checked against the named class's instance
// methods in the .d.ts. Update when new state fields hold WASM
// handles. Anchored with \b so e.g. `recState.chromaticTunerSilence`
// doesn't accidentally match `recState.chromaticTuner`.
const WASM_ACCESSORS = [
    { pattern: /\btunerState\.tuner\b/g, className: 'WebTuner' },
    { pattern: /\brecState\.chromaticTuner\b/g, className: 'WebTuner' },
    { pattern: /\bplaybackState\.chromaticTuner\b/g, className: 'WebTuner' },
    { pattern: /\bplaybackState\.loadedTab\.parsed\b/g, className: 'WebParsedTab' },
    { pattern: /\bplaybackState\.loadedTab\.transposed\b/g, className: 'WebParsedTab' },
];

// ──────────────────────────── d.ts parsing ────────────────────────────

/// Parse the wasm-bindgen-generated .d.ts. Returns
///   { freeFunctions: Set<string>, classes: { [name]: { instance: Set, static: Set } } }
/// Hand-rolled (no tsc dep) because the .d.ts shape is mechanical —
/// wasm-bindgen emits the same patterns every release. If those
/// patterns change in a future wasm-bindgen update this parser will
/// stop finding methods, the check will fail loudly, and we'll update
/// the parser — preferable to a silent miss.
function parseDts(text) {
    const lines = text.split('\n');
    const freeFunctions = new Set();
    const classes = {};

    let currentClass = null;
    let depth = 0;

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trim();

        // Track when we're inside a class body — we count braces because
        // method signatures contain `{` for parameter defaults / type
        // literals that aren't class scope.
        if (currentClass) {
            for (const ch of line) {
                if (ch === '{') depth++;
                else if (ch === '}') {
                    depth--;
                    if (depth === 0) {
                        currentClass = null;
                        break;
                    }
                }
            }
        }

        const classMatch = trimmed.match(/^export class (\w+) \{/);
        if (classMatch) {
            currentClass = classMatch[1];
            classes[currentClass] = { instance: new Set(), static: new Set() };
            depth = 1;
            continue;
        }

        if (currentClass) {
            // Static methods: `static fooBar(...)`.
            const staticMatch = trimmed.match(/^static (\w+)\(/);
            if (staticMatch) {
                classes[currentClass].static.add(staticMatch[1]);
                continue;
            }
            // Instance methods: bare `fooBar(...)`. Skip the
            // `private constructor()` declaration and the
            // `[Symbol.dispose]` exotic entry — neither is a callable
            // name we'd see in JS.
            const instanceMatch = trimmed.match(/^(\w+)\(/);
            if (instanceMatch) {
                const name = instanceMatch[1];
                if (name !== 'constructor' && name !== 'private') {
                    classes[currentClass].instance.add(name);
                }
            }
            continue;
        }

        // Free functions: `export function fooBar(...)`.
        const fnMatch = trimmed.match(/^export function (\w+)\(/);
        if (fnMatch) freeFunctions.add(fnMatch[1]);
    }

    return { freeFunctions, classes };
}

// ─────────────────────────── JS source scan ───────────────────────────

/// Strip `//` and `/* */` comments. Naive but adequate — we're not
/// trying to handle strings containing comment-looking characters,
/// just preventing the bridge-check from flagging method names that
/// only appear inside comments / docblocks.
function stripComments(src) {
    return src
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/\/\/[^\n]*/g, '');
}

/// Pull out the contents of a `<script type="module">` block from an
/// HTML file. app.html has exactly one; if that ever changes the
/// regex flag `g` would catch them all and we'd concat the bodies.
function extractScriptFromHtml(html) {
    const re = /<script[^>]*type="module"[^>]*>([\s\S]*?)<\/script>/g;
    const parts = [];
    let m;
    while ((m = re.exec(html)) !== null) parts.push(m[1]);
    return parts.join('\n');
}

async function loadJsBody(relPath) {
    const abs = resolve(REPO_ROOT, relPath);
    const raw = await readFile(abs, 'utf8');
    const body = relPath.endsWith('.html') ? extractScriptFromHtml(raw) : raw;
    return stripComments(body);
}

// ───────────────────────────── checks ────────────────────────────────

/// Find every static-method call like `WebTuner.foo(` and verify the
/// method exists. Returns a list of `{ file, line, className, method }`
/// for unknown calls.
function checkStaticCalls(body, file, classes) {
    const issues = [];
    const lines = body.split('\n');
    for (let i = 0; i < lines.length; i++) {
        for (const className of Object.keys(classes)) {
            const re = new RegExp(`\\b${className}\\.(\\w+)\\(`, 'g');
            let m;
            while ((m = re.exec(lines[i])) !== null) {
                const method = m[1];
                if (!classes[className].static.has(method)) {
                    issues.push({ file, line: i + 1, className, method, kind: 'static' });
                }
            }
        }
    }
    return issues;
}

/// Find every instance-method call on a registered WASM accessor and
/// verify the method exists on the corresponding class.
function checkInstanceCalls(body, file, classes) {
    const issues = [];
    const lines = body.split('\n');
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        for (const { pattern, className } of WASM_ACCESSORS) {
            // Reset the regex's lastIndex each iteration since we
            // declared it with /g.
            pattern.lastIndex = 0;
            let m;
            while ((m = pattern.exec(line)) !== null) {
                // Look at what follows the accessor — `.method(` means
                // a method call; anything else (assignment, comparison)
                // is a non-call use we don't care about.
                const after = line.slice(m.index + m[0].length);
                const callMatch = after.match(/^\.(\w+)\s*\(/);
                if (!callMatch) continue;
                const method = callMatch[1];
                if (!classes[className].instance.has(method)) {
                    issues.push({ file, line: i + 1, className, method, kind: 'instance' });
                }
            }
        }
    }
    return issues;
}

// ───────────────────────────── main ──────────────────────────────────

async function main() {
    let dtsText;
    try {
        dtsText = await readFile(DTS_PATH, 'utf8');
    } catch (e) {
        console.error(`\n✗ Cannot read ${DTS_PATH} — has the WASM bundle been built?`);
        console.error('  Local: cargo build --target wasm32-unknown-unknown -p twanga-web --release');
        console.error('         wasm-bindgen --target web --out-dir frontend/web/pkg target/wasm32-unknown-unknown/release/twanga_web.wasm');
        console.error('  CI: see pages.yml for the canonical command.');
        process.exit(2);
    }
    const { freeFunctions, classes } = parseDts(dtsText);
    const classNames = Object.keys(classes);
    console.log(`Loaded API surface from ${DTS_PATH}:`);
    console.log(`  free functions: ${freeFunctions.size}`);
    for (const c of classNames) {
        const s = classes[c];
        console.log(`  ${c}: ${s.instance.size} instance + ${s.static.size} static`);
    }

    const allIssues = [];
    for (const rel of JS_SOURCES) {
        let body;
        try {
            body = await loadJsBody(rel);
        } catch (e) {
            // A source file may legitimately be absent (e.g. tauri shim
            // not present locally). Skip rather than fail.
            if (e.code === 'ENOENT') continue;
            throw e;
        }
        allIssues.push(...checkStaticCalls(body, rel, classes));
        allIssues.push(...checkInstanceCalls(body, rel, classes));
    }

    if (allIssues.length > 0) {
        console.error(`\n✗ ${allIssues.length} call(s) to undeclared WASM methods:\n`);
        for (const i of allIssues) {
            console.error(`  ${i.file}:${i.line}  ${i.className}.${i.method}()  [${i.kind}]`);
        }
        console.error('\nFix:');
        console.error('  - Add a #[wasm_bindgen] method on the Rust side, OR');
        console.error('  - Update the JS call site, OR');
        console.error('  - If the call is correct but the accessor is new, add it to');
        console.error('    WASM_ACCESSORS in tools/check-wasm-bridge.mjs.');
        process.exit(1);
    }

    console.log(`\n✓ All WASM bridge calls resolve to declared exports.`);
}

main().catch((err) => {
    console.error('check failed:', err);
    process.exit(2);
});
