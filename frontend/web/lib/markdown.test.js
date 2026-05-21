// Tests for the hand-rolled markdown renderer.
//
// Run with:  node --test frontend/web/lib/markdown.test.js
//
// Pure stdlib — uses Node 18+'s built-in `node:test` and `node:assert`,
// no installed deps. The renderer is a pure function (string in, string
// out) so the test setup is trivial — no jsdom needed.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

import { renderMarkdown } from './markdown.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const FEATURES_DIR = resolve(__dirname, '../../../docs/features');

// ── Headings ──────────────────────────────────────────────────────────

test('h1 / h2 / h3 render as the matching tag', () => {
    assert.match(renderMarkdown('# Top'), /<h1>Top<\/h1>/);
    assert.match(renderMarkdown('## Mid'), /<h2>Mid<\/h2>/);
    assert.match(renderMarkdown('### Low'), /<h3>Low<\/h3>/);
});

test('h4+ collapses to h3 (we only style up to h3)', () => {
    assert.match(renderMarkdown('#### Deeper'), /<h3>Deeper<\/h3>/);
});

// ── Paragraphs ────────────────────────────────────────────────────────

test('plain text becomes a paragraph', () => {
    assert.match(renderMarkdown('Hello world.'), /<p>Hello world\.<\/p>/);
});

test('consecutive non-blank lines join into one paragraph', () => {
    const html = renderMarkdown('Line one.\nLine two.');
    assert.match(html, /<p>Line one\. Line two\.<\/p>/);
});

test('a blank line ends a paragraph', () => {
    const html = renderMarkdown('First.\n\nSecond.');
    assert.match(html, /<p>First\.<\/p>[\s\S]*<p>Second\.<\/p>/);
});

// ── Inline emphasis ───────────────────────────────────────────────────

test('**bold** renders strong', () => {
    assert.match(renderMarkdown('**loud**'), /<strong>loud<\/strong>/);
});

test('*italic* renders em', () => {
    assert.match(renderMarkdown('*soft*'), /<em>soft<\/em>/);
});

test('bold and italic can coexist without bleeding', () => {
    const html = renderMarkdown('**bold** and *italic*');
    assert.match(html, /<strong>bold<\/strong>/);
    assert.match(html, /<em>italic<\/em>/);
});

// ── Inline code ──────────────────────────────────────────────────────

test('inline `code` renders code tags', () => {
    assert.match(renderMarkdown('use `--flag` here'), /<code>--flag<\/code>/);
});

test('inline code escapes HTML inside the span', () => {
    const html = renderMarkdown('like `<b>` here');
    assert.match(html, /<code>&lt;b&gt;<\/code>/);
});

test('code spans suppress emphasis inside them', () => {
    // The `*` inside backticks should NOT trigger <em>.
    const html = renderMarkdown('see `a*b*c`');
    assert.match(html, /<code>a\*b\*c<\/code>/);
    assert.doesNotMatch(html, /<em>/);
});

// ── Fenced code blocks ────────────────────────────────────────────────

test('``` fences render pre/code with escaped content', () => {
    const md = '```\nlet x = "<y>";\n```';
    const html = renderMarkdown(md);
    assert.match(html, /<pre><code>let x = &quot;&lt;y&gt;&quot;;<\/code><\/pre>/);
});

test('fenced code with a language tag adds a lang-* class', () => {
    const html = renderMarkdown('```bash\nls\n```');
    assert.match(html, /<pre><code class="lang-bash">ls<\/code><\/pre>/);
});

// ── Lists ─────────────────────────────────────────────────────────────

test('unordered list with `-` items', () => {
    const html = renderMarkdown('- one\n- two\n- three');
    assert.match(html, /<ul><li>one<\/li><li>two<\/li><li>three<\/li><\/ul>/);
});

test('unordered list with `*` items', () => {
    const html = renderMarkdown('* one\n* two');
    assert.match(html, /<ul><li>one<\/li><li>two<\/li><\/ul>/);
});

test('ordered list with `1.` items', () => {
    const html = renderMarkdown('1. one\n2. two');
    assert.match(html, /<ol><li>one<\/li><li>two<\/li><\/ol>/);
});

test('list items can carry inline emphasis', () => {
    const html = renderMarkdown('- **bold** item');
    assert.match(html, /<li><strong>bold<\/strong> item<\/li>/);
});

// ── Tables ────────────────────────────────────────────────────────────

test('GFM-style pipe table renders thead + tbody', () => {
    const md = [
        '| Col A | Col B |',
        '|-------|-------|',
        '| one   | two   |',
        '| three | four  |',
    ].join('\n');
    const html = renderMarkdown(md);
    assert.match(html, /<table>/);
    assert.match(html, /<th>Col A<\/th><th>Col B<\/th>/);
    assert.match(html, /<td>one<\/td><td>two<\/td>/);
    assert.match(html, /<td>three<\/td><td>four<\/td>/);
});

// ── Links ─────────────────────────────────────────────────────────────

test('links render with href', () => {
    const html = renderMarkdown('see [the page](path.md) yes');
    assert.match(html, /<a href="path\.md">the page<\/a>/);
});

test('javascript: URLs are neutered defensively', () => {
    const html = renderMarkdown('[x](javascript:alert(1))');
    assert.doesNotMatch(html, /javascript:/i);
    assert.match(html, /<a href="#">x<\/a>/);
});

// ── Images ────────────────────────────────────────────────────────────

test('image syntax renders as <img>', () => {
    const html = renderMarkdown('![Tuner screen](screenshots/tuner.png)');
    assert.match(html, /<img src="screenshots\/tuner\.png" alt="Tuner screen">/);
});

test('image with empty alt still produces a valid tag', () => {
    const html = renderMarkdown('![](foo.png)');
    assert.match(html, /<img src="foo\.png" alt="">/);
});

test('image takes precedence over link (no stray "!" leftover)', () => {
    // Without ordering, `![alt](url)` would match the link regex and
    // leave a literal `!` in front of the resulting <a>.
    const html = renderMarkdown('![alt](url)');
    assert.doesNotMatch(html, /^!<a/);
    assert.match(html, /<img /);
});

test('image javascript: URLs are neutered defensively', () => {
    const html = renderMarkdown('![x](javascript:alert(1))');
    assert.doesNotMatch(html, /javascript:/i);
    assert.match(html, /<img src="#" alt="x">/);
});

// ── XSS / escaping ────────────────────────────────────────────────────

test('raw HTML in paragraph text is escaped, not rendered', () => {
    const html = renderMarkdown('hello <script>alert(1)</script>');
    assert.doesNotMatch(html, /<script>/);
    assert.match(html, /&lt;script&gt;/);
});

test('& < > " \' are escaped in paragraph text', () => {
    const html = renderMarkdown('a & b < c > d " e \' f');
    assert.match(html, /&amp;/);
    assert.match(html, /&lt;/);
    assert.match(html, /&gt;/);
});

// ── Blockquotes / HR ──────────────────────────────────────────────────

test('blockquote renders', () => {
    assert.match(renderMarkdown('> note'), /<blockquote>note<\/blockquote>/);
});

test('horizontal rule renders', () => {
    assert.match(renderMarkdown('---'), /<hr>/);
});

// ── Smoke test: every shipped feature page renders without throwing ──

test('every docs/features/*.md renders to non-empty HTML with an H1', async () => {
    const entries = await readdir(FEATURES_DIR);
    const mdFiles = entries.filter((f) => f.endsWith('.md') && f !== 'README.md');
    assert.ok(mdFiles.length >= 5, `expected ≥5 feature pages, found ${mdFiles.length}`);
    for (const name of mdFiles) {
        const md = await readFile(resolve(FEATURES_DIR, name), 'utf8');
        const html = renderMarkdown(md);
        assert.ok(html.length > 0, `${name} rendered empty`);
        assert.match(html, /<h1>/, `${name} should have an H1`);
    }
});
