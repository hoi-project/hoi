<script lang="ts">
  import Identicon from './Identicon.svelte';
  import { avatarUrl } from '../profile-picture';
  import { initials } from '../util';
  import type { AvatarColor } from '@hoi/core-api';
  let {
    name,
    keyHex = '',
    avatar,
    avatarColor,
    accountBadge = false,
    size = 'md',
    ghost = false,
  }: { name: string; keyHex?: string; avatar?: string; avatarColor?: AvatarColor; accountBadge?: boolean; mine?: boolean; size?: 'sm' | 'md' | 'lg' | 'xl'; ghost?: boolean } = $props();
  const pixels = $derived(({ sm: 24, md: 34, lg: 44, xl: 68 })[size]);
  const badgePixels = $derived(Math.max(18, Math.round(pixels * 0.42)));
  const photo = $derived(avatarUrl(avatar));
</script>

<span class={['profile-avatar', ghost && 'ghost']} style:width={`${pixels}px`} style:height={`${pixels}px`} aria-hidden="true">
  {#if ghost}—{:else if photo}<img src={photo} alt="" width={pixels} height={pixels} />{:else}<Identicon {keyHex} color={avatarColor} size={pixels} />{/if}
  {#if accountBadge && !ghost}
    <span class="avatar-initials" style:width={`${badgePixels}px`} style:height={`${badgePixels}px`} style:font-size={`${Math.max(8, Math.round(badgePixels * 0.4))}px`}>{initials(name)}</span>
  {/if}
</span>

<style>
  .profile-avatar { position: relative; display: inline-flex; align-items: center; justify-content: center; flex: 0 0 auto; border-radius: 50%; vertical-align: middle; }
  img { width: 100%; height: 100%; object-fit: cover; border-radius: 50%; }
  .avatar-initials { position: absolute; right: -3px; bottom: -3px; display: inline-flex; align-items: center; justify-content: center; box-sizing: border-box; border: 2px solid var(--card); border-radius: 50%; background: var(--ink); color: var(--card); font-weight: 800; line-height: 1; letter-spacing: -0.02em; }
  .ghost { background: var(--soft); color: var(--mute); border: 1px dashed var(--line); }
</style>
