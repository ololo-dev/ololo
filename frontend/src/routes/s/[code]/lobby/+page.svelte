<script lang="ts">
  import { browser } from "$app/environment";
  import { goto, invalidateAll } from "$app/navigation";
  import { onMount, onDestroy } from "svelte";
  import type { PageData } from "./$types";
  import type { MemberInfo } from "$lib/types/arena";
  import { getToken, initAuth } from "$lib/auth";
  import { patchSession } from "$lib/api";
  import { notify } from "$lib/notifications.svelte";
  import { WsSessionClient } from "$lib/ws-session.svelte";
  import SessionLobby from "$lib/components/session/SessionLobby.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { countdownDigits as toDigits } from "$lib/format";

  let { data }: { data: PageData } = $props();

  const isOwner = $derived(data.user?.id != null && data.user.id === data.session.owner_id);
  const isAdmin = $derived(data.isAdmin ?? false);
  const canControl = $derived(isOwner || isAdmin);

  let controlBusy = $state(false);

  async function cancelSession(): Promise<void> {
    if (controlBusy) return;
    if (!window.confirm("Cancel session? This ends the session for all players.")) return;
    controlBusy = true;
    try {
      await patchSession(data.session.id, { status: "cancelled" }, { fetch });
      notify.success("Session cancelled.");
      await invalidateAll();
    } catch (err) {
      notify.error(`Failed to cancel session: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      controlBusy = false;
    }
  }

  let wsClient = $state<WsSessionClient | null>(null);
  let participants = $state<MemberInfo[]>([]);
  let copied = $state(false);
  let copyTimeout: ReturnType<typeof setTimeout> | null = null;

  const countdownDigits = $derived(toDigits(wsClient?.countdownSecs ?? 0));

  function copyCommand() {
    if (!browser) return;
    navigator.clipboard.writeText(`ololo join ${data.session.join_code}`).then(() => {
      copied = true;
      if (copyTimeout) clearTimeout(copyTimeout);
      copyTimeout = setTimeout(() => { copied = false; }, 2000);
    });
  }

  // The invite link is the public session dashboard (/s/<code>): it shows
  // the project, status, roster and the `ololo join <code>` command a
  // newcomer needs. The command above is for this player's own terminal.
  let inviteCopied = $state(false);
  let inviteTimeout: ReturnType<typeof setTimeout> | null = null;
  function copyInviteLink() {
    if (!browser) return;
    navigator.clipboard
      .writeText(`${window.location.origin}/s/${data.session.join_code}`)
      .then(() => {
        inviteCopied = true;
        if (inviteTimeout) clearTimeout(inviteTimeout);
        inviteTimeout = setTimeout(() => { inviteCopied = false; }, 2000);
      });
  }

  $effect(() => {
    if (!wsClient) return;
    participants = wsClient.participants;
    if (wsClient.phase !== "lobby") {
      goto(`/s/${data.session.join_code}`);
    }
  });

  onMount(async () => {
    await initAuth();
    const token = getToken();
    const client = new WsSessionClient(data.session.join_code, token);
    wsClient = client;
    client.connect();
  });

  onDestroy(() => {
    if (copyTimeout) clearTimeout(copyTimeout);
    wsClient?.disconnect();
  });
</script>

<svelte:head>
  <title>Session lobby — ololo.dev</title>
</svelte:head>

<div class="-mx-6 -mt-8 min-h-screen" style="background: #f4f8fe;">
  <div class="mx-auto max-w-[1206px] px-[18px] py-[48px]">
    <div class="flex flex-col items-start gap-[30px] md:flex-row">

      <!-- ── LEFT: session metadata ──────────────────────────────────────── -->
      <aside class="w-full shrink-0 md:w-[370px]">
        <div class="rounded-[8px] bg-white" style="padding: 40px 32px 0;">

          <!-- Join code heading -->
          <h1 class="mb-[16px] font-heading text-[34px] font-bold leading-tight" style="color: #363636;">
            {data.session.join_code}
          </h1>

          <!-- Join command box with copy button -->
          <div class="mb-[22px]">
            <div
              class="flex items-center justify-between rounded-[4px] font-mono text-sm text-white"
              style="background: #36547f; border-left: 8px solid #fb341c;"
            >
              <code class="px-[20px] py-[14px]">ololo join {data.session.join_code}</code>
              <button
                onclick={copyCommand}
                title="Copy command"
                class="mr-[6px] flex shrink-0 items-center gap-[5px] rounded-[4px] px-[10px] py-[6px] text-[11px] font-sans font-semibold transition-all"
                style="color: {copied ? '#6be597' : 'rgba(255,255,255,0.65)'}; background: {copied ? 'rgba(107,229,151,0.12)' : 'transparent'};"
              >
                {#if copied}
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
                  Copied!
                {:else}
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="8" height="4" x="8" y="2" rx="1" ry="1"></rect><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"></path></svg>
                  Copy
                {/if}
              </button>
            </div>
            <div class="mt-[8px] flex items-center gap-[8px]">
              <button
                onclick={copyInviteLink}
                data-testid="copy-invite-link"
                class="flex items-center gap-[5px] text-[12px] font-semibold transition-colors"
                style="color: {inviteCopied ? '#6be597' : '#4a90e2'};"
              >
                {#if inviteCopied}
                  <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
                  Invite link copied!
                {:else}
                  <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"></path><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path></svg>
                  Copy invite link
                {/if}
              </button>
              <span class="text-[12px]" style="color: #8fb4ec;">
                — send it to invite another player
              </span>
            </div>
          </div>

          <!-- Project link -->
          {#if data.session.project_name}
            <div class="mb-[22px] flex items-start">
              <span
                class="mr-[5px] mt-[2px] w-[72px] shrink-0 text-[12px] font-semibold leading-[1.33]"
                style="color: #8fb4ec;"
              >Project</span>
              <a
                href={data.session.project_slug
                  ? `/projects/${data.session.project_slug}`
                  : data.session.project_id
                    ? `/projects/${data.session.project_id}`
                    : "#"}
                class="flex-grow text-[14px] font-medium hover:underline"
                style="color: #4a90e2;"
              >{data.session.project_name}</a>
            </div>
          {/if}

          {#if browser && canControl}
            <div class="mb-[22px]">
              <Button variant="danger" size="sm" disabled={controlBusy} onclick={cancelSession}>Cancel session</Button>
            </div>
          {/if}

          <!-- Countdown -->
          <div class="mb-[22px]">
            <div class="mb-[6px] text-[12px] font-semibold" style="color: #8fb4ec;">Starts in</div>
            <div class="flex items-center gap-[6px]">
              {#each countdownDigits as digit, i (i)}
                <div
                  class="flex h-[56px] w-[56px] items-center justify-center rounded-[6px] text-[28px] font-bold"
                  style="background: #f4f8fe; color: #8fb4ec;"
                >{digit}</div>
                {#if i < 2}
                  <span class="text-[24px] font-bold" style="color: #8fb4ec;">:</span>
                {/if}
              {/each}
            </div>
          </div>

          <!-- Footer -->
          <div class="mx-[-32px] border-t-2 px-[32px] py-[24px]" style="border-color: #f4f8fe;">
            <span class="text-[12px] font-semibold" style="color: #8fb4ec;">
              {participants.length === 0
                ? "Waiting for participants…"
                : `${participants.length} participant${participants.length === 1 ? "" : "s"} in lobby`}
            </span>
          </div>
        </div>
      </aside>

      <!-- ── RIGHT: player list ──────────────────────────────────────────── -->
      <div class="min-w-0 flex-grow">
        {#if wsClient?.protocolMismatch}
          <div class="mb-[12px] rounded-[8px] bg-white px-[20px] py-[14px] text-[13px]" style="color: #fb341c;">
            Update required. Please refresh the page.
          </div>
        {/if}
        {#if wsClient?.degraded}
          <div class="mb-[12px] rounded-[8px] bg-white px-[20px] py-[14px] text-[13px]" style="color: #8fb4ec;">
            {wsClient.degraded}
          </div>
        {/if}
        <SessionLobby {participants} />
      </div>

    </div>
  </div>
</div>
