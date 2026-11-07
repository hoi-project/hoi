<script lang="ts">
  import { copyText } from '../util';
  import { toast } from '../toast.svelte';
  import { nudge } from '../motion';
  let { label, value, what = 'Copied' }: { label: string; value: string; what?: string } = $props();
  const id = `cf-${Math.random().toString(36).slice(2, 8)}`;
  async function copy(e: MouseEvent) {
    const btn = e.currentTarget as HTMLElement;
    if (await copyText(value)) {
      nudge(btn);
      toast(what);
    } else toast('Copy did not work here. Select the text and copy it yourself.');
  }
</script>

<div class="field">
  <label class="label" for={id}>{label}</label>
  <div class="cf">
    <input {id} class="input mono-in" readonly {value} onfocus={(e) => e.currentTarget.select()} />
    <button type="button" class="pill pill-sm" onclick={copy}>Copy</button>
  </div>
</div>

<style>
  .cf { display: flex; gap: 8px; min-width: 0; }
  .cf .input { flex: 1; text-overflow: ellipsis; }
</style>
