<script lang="ts">
  import { setTitle } from '../lib/app.svelte';
  let { path }: { path: string } = $props();
  setTitle('Nothing here');
</script>

<div class="empty">
  <h1>Nothing lives at {path}.</h1>
  <p>Maybe it faded, or the link got cut off. hoi only shows what this device has.</p>
  <a class="pill pill-ink" href="#/">Home</a>
</div>
