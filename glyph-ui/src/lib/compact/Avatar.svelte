<script lang="ts">
  // The own profile picture (profile.rs own_avatar): the account's picture,
  // else a round placeholder with the user icon.
  import Icon from "../components/Icon.svelte";
  import { app } from "../state.svelte";
  let { size = 40 }: { size?: number } = $props();
</script>

{#if app.account?.avatar}
  <img src={app.account.avatar} alt="" style:width="{size}px" style:height="{size}px" />
{:else}
  <span class="ph" style:width="{size}px" style:height="{size}px"><Icon name="user-round" size={size * 0.48} color="rgba(255,255,255,.7)" /></span>
{/if}

<style>
  img, .ph { flex: none; border-radius: 50%; object-fit: cover; }
  .ph { display: grid; place-items: center; background: #202022; }
</style>
