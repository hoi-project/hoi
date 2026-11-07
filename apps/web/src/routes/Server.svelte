<script lang="ts">
  // A public node, with a registry if you like, for a server: this device rolls its
  // name and makes its files and commands, and hands them to the person; nothing goes anywhere
  // else. The server runs the installer, which checks it first and stops before changing anything
  // on a BLOCK. Everything that decides what the files say is in lib/server-kit.ts.
  import { app, setTitle } from '../lib/app.svelte';
  import { enterView, nudge, popIn } from '../lib/motion';
  import { toast, toastError } from '../lib/toast.svelte';
  import { copyText, saveFile } from '../lib/util';
  import {
    type KitRole,
    archiveName,
    cleanName,
    cloudInit,
    configFileName,
    configText,
    hostOf,
    installerUrlFor,
    kitArchive,
    DEFAULT_PIECES_GB,
    kitFiles,
    kitFolderName,
    kitNotice,
    kitProblems,
    noticeFileName,
    oneLiner,
    randomName,
    registryUrlOf,
  } from '../lib/server-kit';
  import { MAX_NOTICE_BYTES, canonicalNotice, isoDate, noticeFooter, noticeSections, noticeTemplate, noticeTitle } from '../lib/notice';
  import { OPERATOR_RESPONSIBILITY } from '../lib/legal/texts';
  import NoticeEditor from '../lib/ui/NoticeEditor.svelte';
  import NoticeText from '../lib/ui/NoticeText.svelte';

  setTitle('A node on a server');

  // Whoever runs a public node answers for other people's data: said first, here.
  let acknowledged = $state(false);
  // The join notice: the operator's own words, in the client's fixed sentences.
  let notice = $state(noticeTemplate('', isoDate(new Date())));
  let retentionText = $state('7');

  let role = $state<KitRole>('registry');
  let name = $state(randomName());
  let domain = $state('');
  let ownAddress = $state(false);
  let registryUrl = $state('');
  let tls = $state(true);
  let keepHostname = $state(false);
  let indexText = $state('');
  let cap = $state('');
  let holdPieces = $state(true);
  let piecesText = $state(DEFAULT_PIECES_GB);
  let way = $state<'copy' | 'order' | 'direct'>('copy');
  let busy = $state(false);

  // The installer the site that served this app has beside it; the project's own elsewhere.
  const installerUrl = installerUrlFor(typeof location === 'undefined' ? undefined : location.href.split('#')[0]);
  const opts = $derived({
    role,
    name,
    domain,
    registryUrl: ownAddress ? registryUrl : '',
    tls,
    keepHostname,
    indexes: indexText.split(/\s+/).filter(Boolean),
    dailyCapGb: cap,
    holdPiecesGb: holdPieces ? piecesText : '',
    notice: { ...notice, retentionDays: notice.ipLogging ? Number.parseInt(retentionText, 10) || 0 : 0, country: notice.country.trim().toUpperCase() },
  });
  const problems = $derived(kitProblems(opts));
  const problem = (field: string) => problems.find((p) => p.field === field)?.message;
  const shown = $derived(kitNotice(opts));
  const noticeBytes = $derived(canonicalNotice(shown).length);
  const noticeProblems = $derived(problems.filter((p) => p.field.startsWith('notice.')));
  const noticeFile = $derived(noticeFileName(opts));
  const ready = $derived(problems.length === 0);
  const host = $derived(hostOf(cleanName(name.trim())));
  const registry = $derived(registryUrlOf(opts));
  const conf = $derived(configFileName(opts));
  const folder = $derived(kitFolderName(opts));
  const archive = $derived(archiveName(opts));
  // What goes into the kit, for the list; the installer itself is fetched only for the download.
  const files = $derived(ready ? kitFiles(opts, '', installerUrl) : []);
  const tlsOn = $derived(role === 'registry' && tls);
  const registryHost = $derived(registry.replace(/^https:\/\//, '').replace(/[/:].*$/, '') || 'its address');
  // What `install.sh check` prints, shortened: one line per check.
  const sample = $derived([
    '  ok     system      Linux x86_64 (Debian 12), glibc 2.36',
    '  ok     rights      root',
    '  ok     source      hoi-node for x86_64-unknown-linux-gnu',
    '  info   firewall    ufw is on: the install opens UDP 7844',
    ...(role === 'registry' ? [`  warn   dns         ${registryHost} has no DNS record yet`] : []),
    '  BLOCK  ports       UDP 7843 taken by another program',
    'Not ready: 1 thing above (BLOCK) would stop the install.',
  ].join('\n'));

  function reroll(e: MouseEvent) {
    name = randomName();
    nudge(e.currentTarget as Element);
  }

  async function download() {
    busy = true;
    try {
      const installer = (await import('../../../../deploy/install/install.sh?raw')).default;
      const kit = await kitArchive(opts, installer, installerUrl);
      await saveFile(kit.name, kit.bytes);
      toast(`${kit.name}: made on this ${app.platformName}, sent nowhere. Copy it to the server next.`);
    } catch (err) {
      toastError(err);
    }
    busy = false;
  }

  async function saveCloudInit() {
    try {
      await saveFile(`${folder}-cloud-init.yaml`, new TextEncoder().encode(cloudInit(opts, installerUrl)));
    } catch (err) {
      toastError(err);
    }
  }

  async function copy(e: MouseEvent, text: string, what: string) {
    if (await copyText(text)) {
      nudge(e.currentTarget as Element);
      toast(what);
    } else toast('Copy did not work here. Select the text and copy it yourself.');
  }
</script>

{#snippet code(text: string, label: string, what = 'Copied.')}
  <div class="code">
    <div class="code-h"><span>{label}</span><button type="button" class="pill pill-sm" onclick={(e) => copy(e, text, what)} aria-label={`Copy: ${label}`}>Copy</button></div>
    <pre><code>{text}</code></pre>
  </div>
{/snippet}

<section class="view server" use:enterView aria-labelledby="srv-h">
  <div class="vh anim head">
    <h1 id="srv-h">A node on a server.</h1>
    <span class="sticker" use:popIn={2}>made here · sent nowhere</span>
  </div>
  <p class="lead anim">
    A public node carries open communities for whoever asks and passes on the other public nodes it knows; with a registry it also answers
    for public community names. Run one on a small rented server (a VPS) or any computer that stays on. Pick what it should be: this
    {app.platformName} makes the files and the commands, and the server's installer checks the server before it changes anything.
  </p>
  <p class="callout anim">
    Nothing on this page leaves this {app.platformName}. Nothing of yours goes into the files either: not your key, your name or this
    {app.platformName}'s address. The node makes its own key on the server, so the server isn't tied to you.
  </p>

  <div class="vh anim sec"><h2>{OPERATOR_RESPONSIBILITY.title}</h2></div>
  <div class="frame duty anim">
    {#each OPERATOR_RESPONSIBILITY.sections as sec (sec.title)}
      <p><b>{sec.title}</b>{#if sec.text}{' '}{sec.text}{/if}</p>
      {#if sec.bullets}<ul>{#each sec.bullets as b (b)}<li>{b}</li>{/each}</ul>{/if}
    {/each}
    <label class="check ack"><input id="srv-ack" type="checkbox" bind:checked={acknowledged} /> <span>{OPERATOR_RESPONSIBILITY.check}</span></label>
    {#if !acknowledged}<p class="help">Tick it to go on: what the node runs, your join notice, the kit.</p>{/if}
  </div>

  {#if acknowledged}
  <div class="vh anim sec"><h2>What it runs</h2></div>
  <div class="roles anim" role="radiogroup" aria-label="What the server runs">
    <label class="role">
      <input type="radio" name="srv-role" value="public" bind:group={role} />
      <span class="dot bg-mint" aria-hidden="true"></span><b>Public node</b>
      <span>Carries open communities and passes on public nodes. No key, no domain, no certificate.</span>
    </label>
    <label class="role">
      <input type="radio" name="srv-role" value="registry" bind:group={role} />
      <span class="dot bg-sun" aria-hidden="true"></span><b>Public node and registry</b>
      <span>The same, with the public-name API at an https:// address under a domain of yours.</span>
    </label>
  </div>

  <div class="sets anim">
    <div class="set top">
      <label class="l" for="srv-name">Name</label>
      <span class="h">
        The node's label{keepHostname ? '' : ' and the server\'s host name'}. A label, not a secret: there are about 368 million of these, and
        the node's key, made on the server, is what tells nodes apart.
      </span>
      <span class="v col">
        <span class="fieldrow">
          <input id="srv-name" class="input mono-in" bind:value={name} maxlength="40" autocomplete="off" spellcheck="false" />
          <button type="button" class="pill pill-sm" onclick={reroll}>Another name</button>
        </span>
        {#if problem('name')}<span class="help bad" role="alert">{problem('name')}</span>
        {:else if !keepHostname && host !== name.trim()}<span class="help">Host name: <b class="mono">{host}</b></span>{/if}
        <label class="check"><input type="checkbox" bind:checked={keepHostname} /> Leave the server's own host name alone</label>
      </span>
    </div>

    {#if role === 'registry'}
      <div class="set top">
        <label class="l" for={ownAddress ? 'srv-url' : 'srv-domain'}>Address</label>
        <span class="h">
          Where apps and nodes reach the registry. {ownAddress ? 'Its host name has to lead to the server.' : `It becomes the name under your domain: point DNS for it at the server (an A record, and AAAA for IPv6).`}
        </span>
        <span class="v col">
          {#if ownAddress}
            <input id="srv-url" class="input mono-in" bind:value={registryUrl} placeholder="https://names.example.org" autocomplete="off" spellcheck="false" />
            {#if problem('registryUrl')}<span class="help bad" role="alert">{problem('registryUrl')}</span>{/if}
          {:else}
            <span class="fieldrow">
              <span class="mono pre">{host || 'name'}.</span>
              <input id="srv-domain" class="input mono-in" bind:value={domain} placeholder="example.org" autocomplete="off" spellcheck="false" />
            </span>
            {#if problem('domain') && domain.trim()}<span class="help bad" role="alert">{problem('domain')}</span>
            {:else if registry}<span class="help">The registry: <b class="mono">{registry}</b></span>{/if}
          {/if}
          <label class="check"><input type="checkbox" bind:checked={ownAddress} /> Use an address of my own</label>
        </span>
      </div>
      <div class="set top">
        <span class="l">HTTPS</span>
        <span class="h">
          Browsers reach a registry only over https. Caddy on the server can serve the address with a certificate of its own, renewed by
          itself, keeping no access log; it needs TCP 80 and 443.
        </span>
        <span class="v col">
          <label class="check"><input type="checkbox" bind:checked={tls} /> Set up Caddy on the server</label>
          <span class="help">{tls ? 'From the system\'s packages: Debian 12, Ubuntu 24.04 and Fedora have it.' : 'Bring your own TLS proxy in front of 127.0.0.1:7845. The kit has a Caddyfile to start from.'}</span>
        </span>
      </div>
    {/if}

    <div class="set">
      <span class="l">Mailboxes</span>
      <span class="h">
        Private communities keep their messages in mailboxes on a few public nodes in different places. Their devices pick nodes that
        offer them on their own; nobody has to set anything up. The node keeps only sealed pieces it cannot read, and its notice says so.
      </span>
      <span class="v col">
        <label class="check"><input type="checkbox" bind:checked={holdPieces} /> Keep mailboxes and encrypted pieces for others</label>
        {#if holdPieces}
          <label class="field inline" for="srv-pieces"><span>Up to</span><input id="srv-pieces" class="input capin" bind:value={piecesText} inputmode="decimal" autocomplete="off" /><span>GB</span></label>
          {#if problem('holdPiecesGb')}<span class="help bad" role="alert">{problem('holdPiecesGb')}</span>{/if}
        {:else}<span class="help">Private communities can't keep their mailboxes on this node.</span>{/if}
      </span>
    </div>

    <details class="set more">
      <summary class="l">More</summary>
      <div class="wide moregrid">
        <label class="field" for="srv-index">
          <span>Also announce it to these registries</span>
          <textarea id="srv-index" class="textarea mono-in" rows="2" bind:value={indexText} placeholder="https://names.example.net" spellcheck="false"></textarea>
          <span class="help">Optional, one https:// address per line. None is needed: nodes find each other through node links and invites{role === 'registry' ? ', and this one announces itself to its own registry' : ''}.</span>
          {#if problem('indexes')}<span class="help bad" role="alert">{problem('indexes')}</span>{/if}
        </label>
        <label class="field" for="srv-cap">
          <span>Daily cap (GB)</span>
          <input id="srv-cap" class="input capin" bind:value={cap} inputmode="decimal" placeholder="5" autocomplete="off" />
          <span class="help">What it may relay for others in a day (UTC). Empty: hoi-node's 5 GB.</span>
          {#if problem('dailyCapGb')}<span class="help bad" role="alert">{problem('dailyCapGb')}</span>{/if}
        </label>
      </div>
    </details>
  </div>

  <div class="vh anim sec"><h2>Your join notice</h2></div>
  <p class="lead anim">
    Everyone who connects reads this before the node serves them, in these words: yours in the fields, the rest the same on every node.
    It travels in the node's link and in its first word to every stranger. The node itself never logs addresses; say what your server
    or your provider keeps. Your name, your contact and where the server is are optional, but you should give them; whatever you fill
    in is shown to everyone who connects to this node.
  </p>
  <div class="anim"><NoticeEditor bind:notice bind:retentionText {problem} idPrefix="srv" /></div>
  <div class="preview anim">
    <div class="kit-h"><h3>As people read it</h3><span class="tag tag-soft mono">{noticeBytes} of {MAX_NOTICE_BYTES} bytes{noticeBytes <= 1100 ? ' · fits a QR code' : ''}</span></div>
    <NoticeText title={noticeTitle(shown)} sections={noticeSections(shown)} footer={noticeFooter(shown)} />
    {#if problem('notice.notice')}<p class="help bad" role="alert">{problem('notice.notice')}</p>{/if}
    <p class="help">The node reads it again whenever the file changes on the server; people who accepted the old words read the new ones before it serves them again.</p>
  </div>

  <div class="frame kit anim" aria-labelledby="srv-kit">
    <div class="kit-h">
      <h2 id="srv-kit">The kit</h2>
      <span class="tag tag-soft mono">{archive}</span>
    </div>
    {#if ready}
      <ul class="files" aria-label="What is in the kit">
        {#each files as f (f.name)}
          <li><span class="mono fname">{f.name}</span><span class="help">{f.about}</span></li>
        {/each}
      </ul>
      <div class="btnrow">
        <button type="button" class="pill pill-ink pill-lg" onclick={download} disabled={busy}>{busy ? 'Making it…' : 'Download the kit'}</button>
        <span class="help">One file with all of it. Download it again after a change.</span>
      </div>
      <details class="peek">
        <summary>What {conf} says</summary>
        <pre><code>{configText(opts)}</code></pre>
      </details>
    {:else}
      <p class="help">{noticeProblems.length ? `Your join notice first: ${noticeProblems[0].message}` : problems[0]?.message}</p>
    {/if}
  </div>

  <div class="vh anim sec"><h2>Install it</h2></div>
  <div class="seg anim" role="radiogroup" aria-label="How to install it">
    <label><input type="radio" name="srv-way" value="copy" bind:group={way} /><span>Copy the kit over</span></label>
    <label><input type="radio" name="srv-way" value="order" bind:group={way} /><span>When you order a server</span></label>
    <label><input type="radio" name="srv-way" value="direct" bind:group={way} /><span>Straight on the server</span></label>
  </div>
  <div class="way anim">
    {#if !ready}
      <p class="help">Fill in what it runs first.</p>
    {:else if way === 'copy'}
      <p>Download the kit, copy it to the server, unpack it and check the server. Then install: the same command without <code>check</code>.</p>
      {@render code(`scp ${archive} root@SERVER:\nssh root@SERVER\ntar -xf ${archive} && cd ${folder}\n./install.sh check --config ${conf}\n./install.sh --config ${conf}`, 'on your computer, then on the server', 'Commands copied.')}
      <p class="help">The configuration names your notice ({noticeFile}); the installer copies it to <code>/etc/hoi-node/notice.json</code>.</p>
      <p class="help">
        A server without the internet: put the release files for it beside <code>install.sh</code> (<code>manifest.json</code>, its
        <code>.minisig</code> and the hoi-node archive for the server's processor, from the release page or
        <a class="textlink" href="https://hoi.sh/download/" target="_blank" rel="noopener">hoi.sh/download</a>). The installer uses them and
        checks each against the release list. The kit's README says it all again.
      </p>
    {:else if way === 'order'}
      <p>
        Most providers have a field called <b>user data</b>, cloud-init or cloud config where you create a server (DigitalOcean, Hetzner,
        Vultr, Linode, Scaleway, AWS and others). Paste this into it: on its first boot the server writes the configuration and installs the node,
        checking itself first. Log in afterwards and read <code>/var/log/cloud-init-output.log</code>.
      </p>
      {@render code(cloudInit(opts, installerUrl), 'cloud-init.yaml', 'cloud-init copied.')}
      <div class="btnrow"><button type="button" class="pill pill-sm" onclick={saveCloudInit}>Save as a file</button></div>
    {:else}
      <p>Flags take the place of the configuration; your notice still has to be on the server: copy {noticeFile} from the kit over first, then check and install.</p>
      {@render code(`scp ${noticeFile} root@SERVER:\n${oneLiner(opts, installerUrl, true)}\n${oneLiner(opts, installerUrl)} --notice-file ./${noticeFile}`, 'on your computer, then on the server', 'Commands copied.')}
    {/if}
  </div>

  <div class="vh anim sec"><h2>Before it installs</h2></div>
  <div class="two anim">
    <div class="needs">
      <h3>What the server needs</h3>
      <ul>
        <li>64-bit Linux with systemd: Debian 12, Ubuntu 22.04 or newer, Fedora, Raspberry Pi OS. A Mac works too.</li>
        <li>512 MB of memory and 1 GB of disk at least.</li>
        <li>A public IPv4 or IPv6 address, or a router that forwards the ports.</li>
        <li>UDP 7843 and 7844{tlsOn ? ', TCP 80 and 443' : ''} open in the provider's firewall. An active ufw or firewalld on the server is opened by the installer.</li>
        {#if role === 'registry'}<li>DNS for <span class="mono">{registryHost}</span> pointing at the server.</li>{/if}
      </ul>
    </div>
    <div class="needs">
      <h3>What the check looks at</h3>
      <p class="help">
        <code>install.sh check</code> prints one line each: the system and its glibc, systemd, root or sudo, the release files, disk, memory,
        the clock, the ports, the host firewall, the server's addresses{role === 'registry' ? ', DNS for the registry' : ''}{tlsOn ? ', Caddy and ports 80 and 443' : ''}, the host
        name and a node installed before. <b>BLOCK</b> means it can't be installed yet, and says why; the install stops before changing anything.
        <b>warn</b> means it installs, but something won't work as well.
      </p>
      <pre class="sample" aria-label="An example of the check's output"><code>{sample}</code></pre>
    </div>
  </div>
  {/if}
</section>

<style>
  .server { max-width: 980px; }
  .head { gap: 16px; }
  .lead { font-size: var(--type-lead-size); line-height: var(--type-lead-lh); color: var(--mute); max-width: 760px; margin-bottom: 14px; }
  .callout { max-width: 760px; }
  .sec { padding-top: 32px; padding-bottom: 12px; }
  .roles { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-bottom: 18px; }
  .role { position: relative; display: grid; grid-template-columns: auto 1fr; align-items: center; gap: 2px 10px; padding: 12px 14px; border: 2px solid var(--line); border-radius: var(--radius-input); background: var(--card); cursor: pointer; }
  .role input { position: absolute; opacity: 0; inset: 0; margin: 0; cursor: pointer; }
  .role b { font-size: 17px; font-weight: 800; }
  .role span:last-child { grid-column: 2; font-size: 13px; color: var(--mute); font-weight: 500; line-height: 1.4; }
  .role:hover { border-color: var(--ink); }
  .role:has(input:checked) { border-color: var(--ink); box-shadow: 3px 3px 0 var(--ink); }
  .role:has(input:focus-visible) { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .set { grid-template-columns: 160px minmax(0, 1fr) minmax(0, 1.3fr); }
  .set .v { justify-content: flex-start; text-align: left; }
  .col { flex-direction: column; align-items: stretch; gap: 8px; }
  .fieldrow { display: flex; gap: 8px; align-items: center; min-width: 0; }
  .fieldrow .input { flex: 1 1 160px; min-width: 0; }
  .pre { white-space: nowrap; color: var(--ink); }
  .check { display: flex; gap: 10px; align-items: center; font-weight: 700; min-height: 40px; }
  .check input { width: 20px; height: 20px; accent-color: var(--ink); flex: none; }
  .bad { color: var(--ink); font-weight: 700; box-shadow: inset 3px 0 0 var(--coral); padding-left: 10px; }
  .more { display: block; }
  .more summary { cursor: pointer; color: var(--cobalt); min-height: 44px; display: flex; align-items: center; }
  .moregrid { display: grid; grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr); gap: 16px; padding-top: 8px; }
  .capin { max-width: 140px; }
  .field.inline { display: flex; flex-direction: row; align-items: center; gap: 10px; }
  .field.inline .capin { max-width: 90px; }
  .duty { padding: 18px 20px; display: flex; flex-direction: column; gap: 10px; max-width: 760px; }
  .duty p, .duty li { line-height: 1.5; color: var(--mute); font-weight: 500; }
  .duty p b { color: var(--ink); }
  .duty ul { margin: 0 0 0 20px; display: flex; flex-direction: column; gap: 4px; }
  .duty .ack { align-items: flex-start; padding-top: 6px; border-top: 2px dotted var(--line); }
  .duty .ack input { margin-top: 2px; }
  .preview { margin-top: 18px; display: flex; flex-direction: column; gap: 12px; max-width: 760px; }
  .preview h3 { font-size: 20px; letter-spacing: -.03em; }
  .kit { margin-top: 24px; padding: 18px; display: flex; flex-direction: column; gap: 14px; }
  .kit-h { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .kit-h h2 { font-size: 24px; letter-spacing: -.03em; }
  .kit-h .tag { white-space: normal; overflow-wrap: anywhere; }
  .files { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .files li { display: grid; grid-template-columns: 200px minmax(0, 1fr); gap: 4px 16px; padding: 8px 0; border-top: 1px solid var(--line); align-items: baseline; }
  .fname { color: var(--ink); font-weight: 600; overflow-wrap: anywhere; }
  .peek summary { cursor: pointer; font-weight: 700; color: var(--cobalt); min-height: 44px; display: flex; align-items: center; }
  .way { display: flex; flex-direction: column; gap: 12px; padding-top: 14px; max-width: 860px; }
  .way p { line-height: 1.5; font-weight: 500; }
  .code { border: 2px solid var(--ink); border-radius: var(--radius-input); background: var(--card); overflow: hidden; }
  .code-h { display: flex; justify-content: space-between; align-items: center; gap: 8px; padding: 6px 6px 6px 12px; border-bottom: 2px solid var(--line); font-size: 13px; font-weight: 700; color: var(--mute); }
  pre { margin: 0; padding: 12px; font-family: var(--mono); font-size: 13px; line-height: 1.55; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--ink); background: var(--soft); border-radius: var(--radius-input); }
  .code pre { border-radius: 0; background: transparent; }
  code { font-family: var(--mono); font-size: .92em; }
  .two { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 24px; }
  .needs { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .needs h3 { font-size: 18px; font-weight: 800; letter-spacing: -.02em; }
  .needs ul { margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 6px; line-height: 1.45; font-weight: 500; }
  /* The check's lines as it prints them: scrolled in their box, never wrapped. */
  .sample { font-size: 12px; white-space: pre; overflow-x: auto; overflow-wrap: normal; }
  @media (max-width: 860px) {
    .set { grid-template-columns: 1fr; }
    .two, .moregrid { grid-template-columns: 1fr; }
    .files li { grid-template-columns: 1fr; }
  }
  @media (max-width: 520px) {
    .roles { grid-template-columns: 1fr; }
    .fieldrow { flex-wrap: wrap; }
  }
</style>
