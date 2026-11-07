<script lang="ts">
  import { springKnob } from '../motion';
  let {
    checked,
    label,
    disabled = false,
    onchange,
  }: { checked: boolean; label: string; disabled?: boolean; onchange: (next: boolean) => void | Promise<unknown> } = $props();
  let knob: HTMLElement | undefined = $state();
  let first = true;
  $effect(() => {
    const on = checked;
    if (first) {
      first = false;
      return;
    }
    springKnob(knob ?? null, on);
  });
</script>

<button class="sw" type="button" role="switch" aria-checked={checked} aria-label={label} {disabled} onclick={() => onchange(!checked)}>
  <i bind:this={knob}></i>
</button>
