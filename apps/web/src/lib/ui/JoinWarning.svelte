<script lang="ts">
  import Modal from './Modal.svelte';

  let {
    open = $bindable(false),
    oncontinue,
  }: { open?: boolean; oncontinue: () => void } = $props();

  function continueJoining() {
    if (!open) return;
    open = false;
    oncontinue();
  }
</script>

<Modal bind:open title="Turn on your VPN before joining">
  <p>For added privacy, activate your VPN and make sure hoi's traffic goes through it before joining.</p>
  <p class="help">Peers and relays you connect to can see your public IP address. A VPN can show them its address instead, but does not guarantee anonymity.</p>
  <p class="help">If you use a home node, its connections need VPN protection on that node too. The Network &amp; Safety notice you acknowledged says what every connection shares; it is in the <a class="textlink" href="#/privacy">privacy notice</a>.</p>
  <div class="btnrow">
    <button type="button" class="pill" onclick={() => (open = false)}>Not now</button>
    <button type="button" class="pill pill-ink" onclick={continueJoining}>Continue joining</button>
  </div>
</Modal>
