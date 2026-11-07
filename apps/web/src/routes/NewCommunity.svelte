<script lang="ts">
  import type { Admission, CommunityColor, CommunityView } from '@hoi/core-api';
  import { COMMUNITY_COLORS, DEFAULT_LIMITS } from '@hoi/core-api';
  import { app, setTitle } from '../lib/app.svelte';
  import { enterView } from '../lib/motion';
  import { router } from '../lib/router.svelte';
  import { to } from '../lib/router';
  import { RETENTION_PRESETS, formatMs, normalizeCommunityName, parseKey, inviteQuery } from '../lib/util';
  import { toast, toastError, errorText } from '../lib/toast.svelte';
  import { resource } from '../lib/resource.svelte';
  import { matchingCommunities } from '../lib/community-discovery';
  import { REGISTRY_URL, lookupName, publishName, publicNameValid } from '../lib/registry';
  import PrivateStart from '../lib/ui/PrivateStart.svelte';

  let { kind }: { kind?: 'private' } = $props();
  setTitle('Start a community');

  let name = $state('');
  let description = $state('');
  let color = $state<CommunityColor>(COMMUNITY_COLORS[Math.floor(Math.random() * COMMUNITY_COLORS.length)]);
  let retention = $state(DEFAULT_LIMITS.maxRetention);
  let admission = $state<Admission>('open');
  let powBits = $state(DEFAULT_LIMITS.powBits);
  let rules = $state('');
  let moreOwners = $state(false);
  let ownersText = $state('');
  let threshold = $state(1);
  let busy = $state(false);
  let makePublic = $state(Boolean(REGISTRY_URL));
  let separate = $state(false);
  let created = $state.raw<CommunityView | null>(null);
  let registrationError = $state('');

  const directory = resource(() => { void app.rev; void app.net; return app.core.discoverCommunities(); });

  const clean = $derived(normalizeCommunityName(name));
  const matches = $derived(matchingCommunities(clean, app.communities, directory.value ?? []));
  $effect(() => { void clean; separate = false; });
  const ownerLines = $derived(ownersText.split('\n').map((l) => l.trim()).filter(Boolean));
  const parsedOwners = $derived(ownerLines.map((l) => ({ raw: l, key: parseKey(l) })));
  const badOwners = $derived(parsedOwners.filter((o) => !o.key));
  const ownerCount = $derived(1 + (moreOwners ? new Set(parsedOwners.filter((o) => o.key && o.key !== app.me?.rootKey).map((o) => o.key)).size : 0));
  $effect(() => {
    if (threshold > ownerCount) threshold = ownerCount;
    if (threshold < 1) threshold = 1;
  });

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!clean) return toast('Give it a name first');
    if (!created && matches.length && !separate) return toast('Open the existing community, or confirm that you want a separate one.');
    if (makePublic && !publicNameValid(clean)) return toast('Public names start with a letter or digit and use lowercase a–z, 0–9, dots, dashes or underscores.');
    if (makePublic && admission !== 'open') return toast('Public discovery requires open joining. Choose Open, or turn off public registration.');
    if (moreOwners && badOwners.length) return toast(`Line ${ownerLines.indexOf(badOwners[0].raw) + 1} isn't a key. Paste 64 hex characters or a did:key.`);
    busy = true;
    registrationError = '';
    try {
      // A lookup helps avoid accidental duplicates. Only the atomic reservation below decides
      // ownership, and retries reuse the same saved community key after a timeout or conflict.
      if (makePublic && !created) {
        const taken = await lookupName(clean);
        if (taken) throw new Error(`c/${clean} is already registered. Find it in Communities instead of creating a separate copy.`);
      }
      const c = created ?? await app.core.createCommunity({
        name: clean,
        description: description.trim(),
        color,
        limits: { maxRetention: retention, admission, powBits },
        rules: rules.trim() || undefined,
        coOwners: moreOwners ? parsedOwners.map((o) => o.key!).filter((k) => k !== app.me?.rootKey) : undefined,
        threshold: moreOwners ? threshold : undefined,
      });
      created = c;
      await app.refresh();
      if (makePublic) await publishName(await app.core.communityPublication(c.id));
      toast(makePublic ? `c/${c.name} is registered. Posts still need a reachable community node to sync.` : `c/${c.name} is saved locally with an unregistered name. Connect a node to share it.`);
      app.inviteFor = c.id;
      router.go(to.community(c.id));
    } catch (err) {
      registrationError = errorText(err);
      toastError(err);
    }
    busy = false;
  }
</script>

<section class="view" use:enterView aria-labelledby="n-h">
  <div class="vh anim"><h1 id="n-h">Start a community</h1></div>
  <nav class="kinds anim" aria-label="Kind of community">
    <a href={to.start()} class="kind" aria-current={kind ? undefined : 'page'}>
      <span class="dot bg-sky" aria-hidden="true"></span><b>Public</b><span>Anyone can read and join; findable by name</span>
    </a>
    <a href={to.start('private')} class="kind" aria-current={kind === 'private' ? 'page' : undefined}>
      <span class="dot bg-lilac" aria-hidden="true"></span><b>Private</b><span>Invite only, encrypted for its members</span>
    </a>
  </nav>

  {#if kind === 'private'}
    <PrivateStart />
  {:else}
  <p class="callout anim intro">
    Want to post in a community that exists? Join it from <a class="textlink" href={to.communities()}>Communities</a> or an invite: a new one
    with the same name is a separate place. The name is fixed once made.
  </p>

  <form class="form" onsubmit={create}>
    <div class="sets anim">
      <div class="set">
        <label class="l" for="n-name">Name</label>
        <span class="h">Local names can repeat. Only a successful public reservation gives this name to your community in the registry.</span>
        <span class="v namev">
          <input id="n-name" class="input" bind:value={name} maxlength="34" placeholder="bikes-repair" autocomplete="off" required disabled={!!created} />
          {#if clean}<span class="tag tag-{color}">c/{clean}</span>{/if}
        </span>
      </div>
      <div class="set top">
        <span class="l">Public name</span>
        <span class="h">Let people find this community by name. Public communities must allow anyone to join. Your profile and posts are not added to the directory.</span>
        <span class="v ownersv">
          <label class="check"><input type="checkbox" bind:checked={makePublic} disabled={!REGISTRY_URL || !!created} /> Reserve this name online</label>
          {#if !REGISTRY_URL}<span class="help">Public names are unavailable. You can still create a community and share its invite.</span>
          {:else if !makePublic}<span class="help">This name stays unregistered. You can register it later if it is still available.</span>{/if}
        </span>
      </div>
      {#if matches.length && !created}
        <div class="callout conflicts" role="status">
          <b>That name is already in use on a known community.</b>
          <p>Join the existing key to see the same posts. These results cover this device and connected nodes, not the entire network.</p>
          {#each matches as match (match.id)}
            <div class="match">
              {#if match.joined}<a class="pill pill-sm" href={to.community(match.id)}>Open c/{match.name}</a>
              {:else if match.invite}<a class="pill pill-sm" href={to.join(inviteQuery(match.invite) ?? '')}>Join c/{match.name}</a>{/if}
              <code>{match.id}</code>
            </div>
          {/each}
          <label class="check"><input type="checkbox" bind:checked={separate} /> I want a separate community with different posts.</label>
        </div>
      {/if}
      <div class="set top">
        <label class="l" for="n-desc">What it's about</label>
        <span class="h">One or two sentences. It travels with every invite.</span>
        <span class="v"><textarea id="n-desc" class="textarea" rows="3" maxlength="280" bind:value={description} placeholder="Fixing bikes, routes, and where to get parts nearby."></textarea></span>
      </div>
      <div class="set">
        <span class="l" id="n-color">Colour</span>
        <span class="h">Its tag, dot and chip everywhere.</span>
        <span class="v">
          <span class="swatches" role="radiogroup" aria-labelledby="n-color">
            {#each COMMUNITY_COLORS as col (col)}
              <label class="sw-c bg-{col}" title={col}>
                <input type="radio" name="n-color" value={col} bind:group={color} />
                <span class="sr-only">{col}</span>
                {#if color === col}<span class="tick" aria-hidden="true">✓</span>{/if}
              </label>
            {/each}
          </span>
        </span>
      </div>
      <div class="set">
        <span class="l" id="n-ret">Posts fade after</span>
        <span class="h">Every device deletes posts older than this. No archive, on purpose.</span>
        <span class="v">
          <span class="seg" role="radiogroup" aria-labelledby="n-ret">
            {#each RETENTION_PRESETS as p (p.seconds)}
              <label><input type="radio" name="n-ret" value={p.seconds} bind:group={retention} /><span>{p.label}</span></label>
            {/each}
          </span>
        </span>
      </div>
      <div class="set">
        <span class="l" id="n-adm">Joining</span>
        <span class="h">Open: a stamp is enough. Approve: a mod admits each new key before their posts reach everyone. Invites and age proofs come later. Joining rules decide who posts, not who reads: posts here are not encrypted, and every node that carries the community can read them. For posts only members can read, start a Private community.</span>
        <span class="v">
          <span class="seg" role="radiogroup" aria-labelledby="n-adm">
            <label><input type="radio" name="n-adm" value="open" bind:group={admission} /><span>Open</span></label>
            <label><input type="radio" name="n-adm" value="approve" bind:group={admission} /><span>A mod says yes</span></label>
            <label title="coming later"><input type="radio" name="n-adm" value="invite" disabled /><span>Invite · later</span></label>
            <label title="coming later"><input type="radio" name="n-adm" value="age-proof" disabled /><span>Age proof · later</span></label>
          </span>
        </span>
      </div>
      <div class="set">
        <label class="l" for="n-pow">Stamp</label>
        <span class="h">A second of work per post keeps floods out. Higher is calmer and slower.</span>
        <span class="v slider">
          <input id="n-pow" type="range" min="12" max="26" bind:value={powBits} />
          <span class="num">{powBits} bits · ≈ {formatMs(app.stampMs(powBits))}</span>
        </span>
      </div>
      <div class="set top">
        <label class="l" for="n-rules">First rules</label>
        <span class="h">Optional. Plain text, one numbered rule per line. Signed as v1.</span>
        <span class="v"><textarea id="n-rules" class="textarea" rows="4" bind:value={rules} placeholder={'1. Be kind.\n2. No selling.\n3. Photos of other people only with a yes.'}></textarea></span>
      </div>
      <div class="set top">
        <span class="l">More owners</span>
        <span class="h">Owners sign rules, limits and moderator changes. With more than one, you can ask for several signatures, so nobody changes the place alone.</span>
        <span class="v ownersv">
          <label class="check"><input type="checkbox" bind:checked={moreOwners} /> Share ownership</label>
          {#if moreOwners}
            <label class="field">
              <span>Their keys, one per line</span>
              <textarea class="textarea mono-in" rows="3" bind:value={ownersText} placeholder={'did:key:z6Mk…\n3e1b70aa…(64 hex)'}></textarea>
              {#if badOwners.length}<span class="help bad">{badOwners.length} {badOwners.length === 1 ? 'line isn’t' : 'lines aren’t'} a key yet.</span>{/if}
            </label>
            <div class="stepper" role="group" aria-label="Signatures needed">
              <button type="button" class="pill pill-sm" aria-label="Fewer signatures" onclick={() => (threshold = Math.max(1, threshold - 1))} disabled={threshold <= 1}>−</button>
              <span class="num"><b>{threshold}</b> of {ownerCount} owners sign changes</span>
              <button type="button" class="pill pill-sm" aria-label="More signatures" onclick={() => (threshold = Math.min(ownerCount, threshold + 1))} disabled={threshold >= ownerCount}>+</button>
            </div>
          {/if}
        </span>
      </div>
    </div>
    {#if registrationError}<p class="callout" role="alert">{registrationError}</p>{/if}
    {#if created}<p class="callout">Your community is saved locally. Its public name is not confirmed. Retry registration with the same key, or <a href={to.community(created.id)}>open the local community</a>. Edits to this form will not change the saved community.</p>{/if}
    <div class="btnrow anim submit">
      <button type="submit" class="pill pill-coral pill-lg" disabled={busy || !clean || (!created && matches.length > 0 && !separate)}>{busy ? (makePublic ? 'Creating and reserving…' : 'Making its key…') : created ? 'Retry public registration' : `Make c/${clean || '…'}`}</button>
      <a class="pill pill-lg" href="#/">Cancel</a>
    </div>
  </form>
  {/if}
</section>

<style>
  .intro { max-width: 720px; margin-bottom: 18px; }
  .kinds { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; max-width: 720px; margin-bottom: 18px; }
  .kind { display: grid; grid-template-columns: auto 1fr; align-items: center; gap: 2px 10px; padding: 12px 14px; border: 2px solid var(--line); border-radius: var(--radius-input); background: var(--card); }
  .kind b { font-size: 17px; font-weight: 800; }
  .kind span:last-child { grid-column: 2; font-size: 13px; color: var(--mute); font-weight: 500; }
  .kind:hover { border-color: var(--ink); }
  .kind[aria-current='page'] { border-color: var(--ink); box-shadow: 3px 3px 0 var(--ink); }
  @media (max-width: 520px) { .kinds { grid-template-columns: 1fr; } }
  .form { display: flex; flex-direction: column; gap: 20px; }
  .set { grid-template-columns: 180px minmax(0, 1fr) minmax(0, 1.3fr); }
  .set .v { justify-content: flex-start; text-align: left; }
  .namev { flex-wrap: wrap; }
  .namev .input { flex: 1 1 180px; }
  .swatches { display: flex; gap: 8px; flex-wrap: wrap; }
  .sw-c { position: relative; width: 44px; height: 44px; border-radius: 14px; border: 2px solid var(--ink); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .sw-c input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
  .sw-c:has(input:checked) { box-shadow: 3px 3px 0 var(--ink); transform: translate(-1px, -1px); }
  .sw-c:has(input:focus-visible) { outline: 3px solid var(--cobalt); outline-offset: 2px; }
  .tick { font-weight: 900; color: var(--tagink); }
  .slider { flex-wrap: wrap; }
  .slider input { flex: 1 1 160px; }
  .ownersv { flex-direction: column; align-items: stretch; gap: 10px; }
  .check { display: flex; gap: 10px; align-items: center; font-weight: 700; min-height: 40px; }
  .check input { width: 20px; height: 20px; accent-color: var(--ink); }
  .stepper { display: flex; align-items: center; gap: 10px; }
  .stepper .pill { min-width: 44px; font-size: 18px; }
  .bad { color: var(--ink); font-weight: 700; }
  .submit { padding-top: 4px; }
  .conflicts { display: flex; flex-direction: column; gap: 12px; }
  .match { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .match code { overflow-wrap: anywhere; font-size: 12px; }
  @media (max-width: 860px) {
    .set { grid-template-columns: 1fr; }
  }
</style>
