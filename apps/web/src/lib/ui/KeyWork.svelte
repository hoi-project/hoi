<script lang="ts">
  // You → Key work: what it is, how much this key carries, and more of it with a live count of
  // tries and a Stop button. Identity work is luck, so the bar shows tries against the usual
  // count for that many bits, never a promise of when it ends.
  import { app } from '../app.svelte';
  import { keyWork, workOutlook, STEP } from '../key-work.svelte';
  import { formatMs } from '../util';

  const me = $derived(app.me!);
  let now = $state(Date.now());
  $effect(() => {
    if (!keyWork.running) return;
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });
  const elapsed = $derived(keyWork.running ? Math.max(0, now - keyWork.started) : 0);
  const outlook = $derived(workOutlook(keyWork.target, keyWork.progress, elapsed));
  const clock = (ms: number) => `${Math.floor(ms / 60_000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}`;
</script>

<div class="set top keywork">
  <span class="l">Key work</span>
  <span class="h">
    Proof that your device worked to make this key. Communities can ask for a minimum, which makes mass-made spam keys too costly.
    It stays with this key; a new key starts from zero.
  </span>
  <span class="v">
    <span class="tag tag-sun big num">{me.identityBits} bits</span>
    {#if me.hasRoot && !me.burned && !keyWork.running}
      <button type="button" class="lnk lnk-ink" onclick={() => keyWork.start()} title="Each step takes about 4× as long as the one before">+{STEP} bits</button>
    {/if}
  </span>
  {#if keyWork.running}
    <div class="wide work" role="status" aria-live="polite">
      <div class="workhead">
        <b>Working towards {keyWork.target} bits…</b>
        <button type="button" class="pill pill-sm pill-danger" onclick={() => keyWork.stop()}>Stop</button>
      </div>
      <div class="bar" role="progressbar" aria-label="Tries against the usual count" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(Math.min(1, outlook.share) * 100)}>
        <i style:width={`${Math.min(0.97, outlook.share) * 100}%`}></i>
      </div>
      <p class="facts">
        <span><b class="num">{keyWork.progress.tries.toLocaleString('en')}</b> of about <b class="num">{outlook.usual.toLocaleString('en')}</b> tries</span>
        <span>best <b class="num">{keyWork.progress.best}</b>/{keyWork.target} bits</span>
        <span class="num">{clock(elapsed)}</span>
        {#if outlook.leftMs !== null}<span>about {formatMs(outlook.leftMs)} left</span>{:else if outlook.share >= 1}<span>unlucky so far, any moment now</span>{/if}
      </p>
      <p class="help">Luck decides when it ends. The rest of hoi keeps working meanwhile, and stopping keeps your {me.identityBits} bits.</p>
    </div>
  {/if}
</div>

<style>
  .keywork .v .tag { font-variant-numeric: tabular-nums; }
  .work { display: flex; flex-direction: column; gap: 8px; background: var(--soft); border-radius: var(--radius-input); padding: 12px 14px; }
  .workhead { display: flex; align-items: center; justify-content: space-between; gap: 10px; flex-wrap: wrap; }
  .bar { height: 10px; border-radius: 999px; background: var(--card); border: 2px solid var(--ink); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--sun); border-right: 2px solid var(--ink); transition: width .4s ease; }
  .facts { display: flex; flex-wrap: wrap; gap: 4px 14px; font-size: 14px; color: var(--mute); font-weight: 500; }
  .facts b { color: var(--ink); font-weight: 800; }
  @media (prefers-reduced-motion: reduce) { .bar i { transition: none; } }
</style>
