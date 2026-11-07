<script lang="ts">
  import type { CommunityRef, Invite, InviteAddress } from '@hoi/core-api';
  import { plural } from '../util';
  import { app, attempt } from '../app.svelte';
  import { toast } from '../toast.svelte';
  import Modal from './Modal.svelte';
  import QR from './QR.svelte';
  import CopyField from './CopyField.svelte';

  let {
    open = $bindable(false),
    community,
    fresh = false,
    onclose,
  }: { open?: boolean; community: CommunityRef; fresh?: boolean; onclose?: () => void } = $props();
  let invite = $state.raw<Invite | null>(null);
  let which = $state<'web' | 'app'>('web');
  const canShare = typeof navigator !== 'undefined' && 'share' in navigator;
  const encoded = $derived(invite?.web.includes('#/join?code=') ?? false);

  $effect(() => {
    if (open && !invite) void load();
  });

  async function load() {
    invite = (await attempt(() => app.core.invite(community.id))) ?? null;
  }

  /** Whose an address is and how far it reaches, in words. */
  function said(a: InviteAddress): string {
    if (a.whose === 'node') return a.scope === 'internet' ? 'Public node — has a copy or has been asked to keep one' : 'Node on a private network';
    if (a.whose === 'entry') return 'Public node — helps find other nodes if the first ones are offline';
    if (a.scope === 'internet') return `your ${app.platformName === 'browser' ? 'home node' : app.platformName}'s internet address`;
    if (a.scope === 'network') return `your ${app.platformName === 'browser' ? 'home node' : app.platformName}, on your home network only`;
    return 'this device only';
  }

  // Nobody outside can use an invite until a public node holds the community: ask the ones
  // this device reaches, then look again.
  let asking = $state(false);
  async function askCarry() {
    asking = true;
    const r = await attempt(() => app.core.requestCarry(community.id));
    if (r) toast(r.asked ? `Asked ${plural(r.asked, 'public node')}. The invite updates once one has it.` : r.carrying ? 'A connected public node keeps it already.' : 'No connected public node takes requests right now. Open a node link under Communities, then try again.');
    setTimeout(() => { asking = false; void load(); }, 3000);
  }

  async function share() {
    if (!invite) return;
    try {
      await navigator.share({ title: `c/${community.name} on hoi`, text: `Come join c/${community.name}.`, url: invite.web });
    } catch {
      /* cancelled */
    }
  }
</script>

<Modal bind:open wide {onclose} title={fresh ? `c/${community.name} exists. Invite someone.` : `Invite a friend to c/${community.name}`}>
  <p class="lead">
    Share <b>c/{community.name}</b> with a link or QR code. The invite includes verified community details
    and connection information to help someone find a copy.
  </p>
  {#if invite}
    <div class="inv">
      <div class="qrcol">
        <QR value={which === 'web' ? invite.web : invite.uri} label={`QR code with the ${which === 'web' ? 'web' : 'app'} invite link`} color={community.color} />
        <p class="help">Scan to open the invite</p>
      </div>
      <div class="links">
        <div class="seg" role="radiogroup" aria-label="Invite link type">
          <label><input type="radio" name="qrwhich" value="web" bind:group={which} /><span>Browser</span></label>
          <label><input type="radio" name="qrwhich" value="app" bind:group={which} /><span>Installed app</span></label>
        </div>
        <CopyField label={which === 'web' ? 'Web link' : 'App link'} value={which === 'web' ? invite.web : invite.uri} what="Invite link copied" />
        <p class="help">{which === 'web' ? 'Opens in a browser. No installation needed.' : 'Opens the hoi app on a computer or phone.'}</p>
        <p class="privacy">{encoded ? 'The link obscures connection addresses, but anyone with it can recover them.' : 'Anyone with this link can read the connection addresses it contains.'}</p>
        {#if canShare}<button type="button" class="pill share" onclick={share}>Share web link…</button>{/if}
      </div>
    </div>
    {#if invite.addresses}
      <section class="inside" aria-label="What this invite contains">
        <h3>Keeping the invite useful</h3>
        <p class="help">For an open community, creating this invite also asks connected public nodes to keep a copy. Once they have it, they can share it while you’re offline.</p>
        {#if !invite.outside}
          <p class="callout">
            This invite has no known connection address that works outside your network yet.
            <button type="button" class="pill pill-sm" onclick={askCarry} disabled={asking}>{asking ? 'Asking…' : 'Ask public nodes to keep a copy'}</button>
          </p>
        {/if}
        {#if invite.addresses.length}
          <details class="addresses">
            <summary>Connection addresses in this invite ({invite.addresses.length})</summary>
            <p class="help">The receiving device needs these addresses to connect. Encoding a link does not make the connection anonymous.</p>
            <ul>
              {#each invite.addresses as a (a.address)}
                <li><span class="mono">{a.address}</span><span class="help">{said(a)}</span></li>
              {/each}
            </ul>
          </details>
        {/if}
      </section>
    {/if}
  {:else}
    <p class="loading">Preparing the invite…</p>
  {/if}
</Modal>

<style>
  .lead { color: var(--mute); font-weight: 500; line-height: 1.5; }
  .lead b { color: var(--ink); }
  .inv { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 20px; align-items: start; }
  .qrcol { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .links { display: flex; flex-direction: column; gap: 14px; min-width: 0; }
  .privacy { padding: 12px 14px; border-radius: var(--radius-input); background: var(--soft); font-size: 13px; line-height: 1.5; }
  .share { align-self: flex-start; }
  .inside { margin-top: 18px; padding-top: 16px; border-top: 1px solid var(--line); display: grid; gap: 8px; }
  .inside h3 { font-size: 15px; margin: 0; }
  .addresses summary { cursor: pointer; font-size: 13px; font-weight: 700; color: var(--cobalt); padding-block: 8px; }
  .addresses ul { margin: 12px 0 0; padding: 0; list-style: none; display: grid; gap: 10px; }
  .addresses li { display: grid; gap: 2px; font-size: 13px; }
  .inside .mono { overflow-wrap: anywhere; }
  @media (max-width: 600px) {
    .inv { grid-template-columns: 1fr; }
    .links { order: -1; }
  }
</style>
