# The hoi wiki

Detailed documentation for the protocol and every program, published with the website at
`/wiki/`. Pages are plain HTML files using the site's partials, with a few build-time blocks.
Everything a page says must be true of the code in this repository: link the source it
describes, and say "the code does X", not "we plan X".

- `nav.json`: the table of contents. Adding a page there adds it to the sidebar, the pager,
  the search index, the build and the sitemap. Its file is `wiki/<slug>/index.html`.
- `plugins/wiki.ts`, `plugins/wiki-blocks.ts`: the layout and the blocks below, at build time.
- `plugins/sbom.ts`: the SBOM page's tables and the CycloneDX file, from the lockfiles.
- `src/pages/wiki.ts`, `src/wiki/`: optional behaviour (search, highlighting, demos, motion).
- `src/wiki/assistant*.ts`: section explanations on eligible browsers. The full
  `virtual:wiki-passages` corpus loads on interaction; a versioned, single-use tab-storage
  handoff carries an explanation to another section. Test the flow with
  `node apps/site/scripts/e2e-assistant.mjs` after building the site.
- `src/pages/playground.ts`, `src/playground/`: the network playground.
- `node scripts/wiki-check.mjs` checks every page (structure, links, CSP rules).

## A page

```html
<!doctype html>
<html lang="en" data-theme="dark">
<head>
  <!-- @include head.html title="Records & encoding · hoi wiki" description="One sentence." -->
</head>
<body class="wiki">
<!-- @include header.html nav="wiki" -->
<!-- @wiki-open -->
<article class="wk-doc">
  <!-- @wiki-crumbs -->
  <h1>Records <span class="acc">&amp; encoding</span></h1>
  <p class="wk-lead">Two or three sentences: what this page explains.</p>
  <p class="wk-src-list">Source <a href="{{REPO_URL}}/blob/main/core/crates/hoi-types/src/record.rs">hoi-types/src/record.rs</a></p>
  <div class="wk-head-rule"></div>

  <section class="wk-sec" id="envelope">
    <h2>The envelope</h2>
    <p>Running text…</p>
    <h3 id="envelope-fields">Fields</h3>
    …
  </section>
</article>
<!-- @wiki-close -->
<!-- @include footer.html -->
<script type="module" src="/src/pages/wiki.ts"></script>
</body>
</html>
```

`<!-- @wiki-open full -->` drops the right-hand "On this page" column (the playground uses it).
"On this page" lists every `<section class="wk-sec" id>`'s `<h2>` and every `<h3 id>`.

With scripts enabled, the playground fills the viewport and hides the wiki chrome. Its
Network, Setup, Details, Activity and Guide views keep controls within reach on phones;
desktop shows the network alongside the selected panel. The guide and wiki navigation
remain ordinary page content without scripts. Playground styling lives in
`src/styles/playground.css`.

## Rules

- No `style` attributes, `<style>` or inline scripts: the Content-Security-Policy forbids them.
  Colours come from classes (`c-sun`, `g-mint`, `t-mint`…).
- `{{REPO_URL}}`, `{{APP_URL}}` and `{{SITE_URL}}` are the only template variables. Never write
  `{{` anywhere else (escape it as `&#123;&#123;` in code samples).
- Escape `<`, `>` and `&` in text and code (`&lt;`, `&gt;`, `&amp;`).
- Links in running text go inside `<p>`, `<li>`, `<td>` or `<dd>` (phones need 44px targets for
  anything else). Wiki links are `/wiki/<slug>/` or `/wiki/<slug>/#<id>`; both are checked.
- No email addresses, no deployment hints, no planning or roadmap: describe what ships.
- Every `<table>` sits in `<div class="wk-table" tabindex="0" role="region" aria-label="…">`.

## Parts

| Part | Markup |
| --- | --- |
| Code | `<div class="wk-code" data-lang="rust" data-title="hoi-types/src/record.rs"><pre><code>…</code></pre></div>` — languages: rust, ts, js, json, sh, toml, yaml, kotlin, swift, cddl, text |
| Table | `<div class="wk-table" tabindex="0" role="region" aria-label="Kinds"><table>…</table></div>` |
| Callout | `<div class="wk-note info"><b>Note</b><p>…</p></div>` — also `tip`, `warn`, `limit` |
| Fields | `<dl class="wk-defs"><dt>created_at<small>u64</small></dt><dd>…</dd></dl>` |
| Cards | `<div class="wk-cards"><a class="wk-card g-mint" href="/wiki/sync/"><i></i><b>Sync</b><span>…</span></a></div>` |
| Steps | `<ol class="wk-steps"><li><b>Title</b><p>…</p></li></ol>` |
| Tags | `<span class="tag t-mint">post</span>` — t-mint, t-sky, t-lilac, t-peach, t-sun, t-coral, t-soft |
| Figure | `<figure class="wk-fig"><svg viewBox="…" role="img" aria-label="…">…</svg><figcaption>…</figcaption></figure>` — classes `d-box`, `d-box acc`, `d-text`, `d-sub`, `d-tag`, `d-line`, `d-line dash`, `d-flow`, `d-dot`, `d-head`, with `c-<colour>` |
| Demo | `<div class="wk-stage" data-demo="stamp" data-title="Mining a stamp"><p class="nojs">What it shows, for readers without scripts.</p><p class="wk-stage-cap">Caption.</p></div>` |

### Sequence diagrams

Written as text, rendered at build time (a numbered list on phones):

```html
<div class="wk-seq" data-title="Opening a node link">
lanes: Browser | Home node | Public node
-- Connect --
Browser -> Home node: WebSocket upgrade to <code>/link</code>
Home node --> Browser: <code>hello</code> {v, caps}
Browser => Home node @coral: <code>recs</code> [records…]
Home node: checks every record
Browser .. Public node: no direct path
</div>
```

`->` request, `-->` reply (dashed), `=>` bulk data, `@colour` before the colon recolours the arrow,
`A: text` is a note on lane A, `A .. B: text` spans lanes, `-- label --` starts a phase. 2–5 lanes.

### Byte layouts

```html
<div class="wk-bytes" data-title="What is hashed for a stamp">
id | 32 B | sun | 4 | BLAKE3 of the canonical record
nonce | 8 B | coral | 1 | what the miner varies
</div>
```

One box per line: `name | size | colour | width (1–6, 8, 10, 12) | description`. A blank line
starts a new row.

### Demos

| `data-demo` | Shows |
| --- | --- |
| `hash` | editing a field changes the canonical bytes, the id and the signature check |
| `stamp` | searching nonces until the hash has enough leading zero bits; a difficulty slider |
| `pipeline` | records passing the validation stages, and where bad ones stop |
| `reconcile` | two devices comparing range fingerprints and splitting until they find what differs |
| `gossip` | a record spreading hop by hop through a mesh, each device checking it first |
| `shamir` | a key split 3-of-5; any three pieces bring it back, two don't |
| `scatter` | an object sealed, transformed, split into data and parity pieces, some lost, rebuilt |
| `ladder` | a newcomer earning vouches, becoming a member, established, trusted, and fading |
| `expiry` | records on a timeline fading at `expires_at`, the tombstone staying |
| `ratelimit` | a key's posts against rolling rate windows; the over-budget ones dropped |
