<script lang="ts">
  import { ikAvatar } from '$lib/imagekit';
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { notify } from '$lib/notifications.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import UserFormModal from '$lib/components/UserFormModal.svelte';
  import { deleteAdminUser, ApiError, type AdminUserDto } from '$lib/api';

  let { data } = $props();

  let userFormOpen = $state(false);
  let editingUser = $state<AdminUserDto | null>(null);
  let deleteTargetUser = $state<AdminUserDto | null>(null);
  let deleting = $state(false);
  let deleteError = $state<string | undefined>(undefined);

  type SortCol = 'display_name' | 'email' | 'is_admin' | 'created_at' | 'usage';
  type SortDir = 'asc' | 'desc';
  let sortCol = $state<SortCol>('created_at');
  let sortDir = $state<SortDir>('asc');

  const sortedUsers = $derived(
    [...data.users].sort((a, b) => {
      let cmp = 0;
      if (sortCol === 'display_name') {
        cmp = a.display_name.localeCompare(b.display_name, undefined, { sensitivity: 'base' });
      } else if (sortCol === 'email') {
        cmp = a.email.localeCompare(b.email, undefined, { sensitivity: 'base' });
      } else if (sortCol === 'is_admin') {
        cmp = (b.is_admin ? 1 : 0) - (a.is_admin ? 1 : 0);
      } else if (sortCol === 'usage') {
        cmp = b.judge_runs_this_month - a.judge_runs_this_month;
      } else {
        cmp = a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0;
      }
      return sortDir === 'asc' ? cmp : -cmp;
    })
  );

  function toggleSort(col: SortCol) {
    if (sortCol === col) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortCol = col;
      sortDir = 'asc';
    }
  }

  function openAddUser() {
    editingUser = null;
    userFormOpen = true;
  }

  function openEditUser(user: AdminUserDto) {
    editingUser = user;
    userFormOpen = true;
  }

  function openDeleteUser(user: AdminUserDto) {
    deleteTargetUser = user;
    deleteError = undefined;
  }

  async function handleDeleteConfirm() {
    if (!deleteTargetUser || deleting) return;
    deleting = true;
    deleteError = undefined;
    const target = deleteTargetUser;
    try {
      await deleteAdminUser(target.id);
      await invalidateAll();
      notify.success(`${target.display_name} has been deleted.`, 'Users');
      deleteTargetUser = null;
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.code === 'cannot_delete_self') {
          deleteError = 'You cannot delete your own account.';
        } else if (err.code === 'has_related_records') {
          deleteError = 'This user owns projects or has active agents and cannot be deleted.';
        } else {
          deleteError = 'An error occurred. Please try again.';
        }
      } else {
        deleteError = 'An unexpected error occurred.';
      }
    } finally {
      deleting = false;
    }
  }

  function formatDate(iso: string): string {
    const d = new Date(iso);
    const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
    return `${months[d.getUTCMonth()]} ${d.getUTCDate()}, ${d.getUTCFullYear()}`;
  }
</script>

<div class="mt-8">
  <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 class="font-heading text-[20px] font-semibold text-brand-text">
        Registered Users
      </h2>
      <p class="mt-0.5 text-sm text-brand-muted">
        {data.users.length} {data.users.length === 1 ? 'user' : 'users'} registered on this instance.
      </p>
    </div>
    <button
      type="button"
      onclick={openAddUser}
      class="shrink-0 rounded-btn bg-brand-blue px-5 py-2 text-sm font-semibold text-white
             transition-opacity hover:opacity-80"
    >
      Add User
    </button>
  </div>

  <div class="rounded-[8px] bg-white shadow-sm">
    {#if data.users.length === 0}
      <div class="flex flex-col items-center justify-center py-16 text-brand-muted">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" class="mb-3 opacity-40" aria-hidden="true">
          <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          <circle cx="9" cy="7" r="4" stroke="currentColor" stroke-width="2"/>
          <path d="M23 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
        <p class="text-sm">No users registered yet.</p>
      </div>
    {:else}
      <!-- Cards below `xl`, like the sessions list: five columns plus two
           action buttons need ~550px, which a phone does not have. -->
      <ul class="divide-y divide-brand-border/60 xl:hidden">
        {#each sortedUsers as user (user.id)}
          {@const initials = (() => {
            const parts = user.display_name.trim().split(/\s+/);
            if (parts.length >= 2) return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
            return user.display_name.slice(0, 2).toUpperCase();
          })()}
          <li class="flex flex-col gap-2.5 p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="flex min-w-0 items-center gap-3">
                {#if user.avatar_url}
                  <img
                    src={ikAvatar(user.avatar_url, 32)}
                    alt=""
                    class="h-8 w-8 shrink-0 rounded-full object-cover"
                  />
                {:else}
                  <div
                    class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full
                           bg-brand-blue/10 text-[11px] font-bold text-brand-blue"
                  >
                    {initials}
                  </div>
                {/if}
                <div class="min-w-0">
                  <div class="flex items-baseline gap-2">
                    <span class="truncate text-sm font-medium text-brand-text">{user.display_name}</span>
                    {#if user.username}
                      <a
                        href="/u/{user.username}"
                        class="shrink-0 font-mono text-xs text-brand-blue hover:underline"
                      >@{user.username}</a>
                    {/if}
                  </div>
                  <div class="truncate text-xs text-brand-muted" title={user.email}>{user.email}</div>
                </div>
              </div>
              {#if user.is_admin}
                <span
                  class="inline-flex shrink-0 items-center rounded-full bg-brand-blue/10 px-2.5 py-0.5
                         text-[11px] font-semibold text-brand-blue"
                >
                  Admin
                </span>
              {/if}
            </div>

            <dl class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 text-xs text-brand-muted">
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Runs</dt>
              <dd
                class="text-[11px] {user.judge_runs_this_month >= user.judge_run_limit_effective
                  ? 'font-semibold text-brand-red'
                  : 'text-brand-muted'}"
                title="Judge runs used this month / monthly limit{user.judge_run_limit != null ? ' (per-user override)' : ''}"
              >
                {user.judge_runs_this_month} / {user.judge_run_limit_effective} runs{user.judge_run_limit != null ? ' *' : ''}{user.judge_run_credits > 0 ? ` +${user.judge_run_credits}` : ''}
              </dd>
              <dt class="font-semibold uppercase tracking-wider text-brand-muted/70">Joined</dt>
              <dd>{formatDate(user.created_at)}</dd>
            </dl>

            <div class="flex justify-end gap-1">
              <button
                type="button"
                onclick={() => openEditUser(user)}
                class="rounded px-2.5 py-1 text-xs font-semibold text-brand-blue
                       transition-colors hover:bg-brand-blue/10"
              >
                Edit
              </button>
              <button
                type="button"
                onclick={() => openDeleteUser(user)}
                class="rounded px-2.5 py-1 text-xs font-semibold text-red-500
                       transition-colors hover:bg-red-50"
              >
                Delete
              </button>
            </div>
          </li>
        {/each}
      </ul>

      <div class="hidden xl:block">
        <table class="w-full">
          <thead>
            <tr class="border-b border-brand-border bg-brand-light-blue/40">
              {#snippet sortTh(col: SortCol, label: string, align?: string)}
                <th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                  <button
                    type="button"
                    onclick={() => toggleSort(col)}
                    class="inline-flex items-center gap-1 transition-colors hover:text-brand-text"
                  >
                    {label}
                    <span class="inline-flex flex-col gap-px text-[7px] leading-none">
                      <span class={sortCol === col && sortDir === 'asc' ? 'text-brand-blue' : 'opacity-30'}>▲</span>
                      <span class={sortCol === col && sortDir === 'desc' ? 'text-brand-blue' : 'opacity-30'}>▼</span>
                    </span>
                  </button>
                </th>
              {/snippet}
              {@render sortTh('display_name', 'User')}
              {@render sortTh('is_admin', 'Role')}
              {@render sortTh('usage', 'Judge runs')}
              {@render sortTh('created_at', 'Joined')}
              <th class="px-4 py-3 text-right text-[11px] font-semibold uppercase tracking-wider text-brand-muted">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {#each sortedUsers as user (user.id)}
              {@const initials = (() => {
                const parts = user.display_name.trim().split(/\s+/);
                if (parts.length >= 2) return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
                return user.display_name.slice(0, 2).toUpperCase();
              })()}
              <tr class="border-b border-brand-border/60 last:border-0 hover:bg-brand-light-blue/30 transition-colors">
                <td class="px-4 py-3">
                  <div class="flex items-center gap-3">
                    {#if user.avatar_url}
                      <img
                        src={ikAvatar(user.avatar_url, 32)}
                        alt=""
                        class="h-8 w-8 shrink-0 rounded-full object-cover"
                      />
                    {:else}
                      <div
                        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full
                               bg-brand-blue/10 text-[11px] font-bold text-brand-blue"
                      >
                        {initials}
                      </div>
                    {/if}
                    <div class="min-w-0">
                      <div class="flex items-baseline gap-2">
                        <span class="truncate text-sm font-medium text-brand-text">{user.display_name}</span>
                        {#if user.username}
                          <a
                            href="/u/{user.username}"
                            class="shrink-0 font-mono text-xs text-brand-blue hover:underline"
                          >@{user.username}</a>
                        {/if}
                      </div>
                      <div class="truncate text-xs text-brand-muted" title={user.email}>{user.email}</div>
                    </div>
                  </div>
                </td>
                <td class="px-4 py-3">
                  {#if user.is_admin}
                    <span
                      class="inline-flex items-center rounded-full bg-brand-blue/10 px-2.5 py-0.5
                             text-[11px] font-semibold text-brand-blue"
                    >
                      Admin
                    </span>
                  {:else}
                    <span
                      class="inline-flex items-center rounded-full bg-gray-100 px-2.5 py-0.5
                             text-[11px] font-semibold text-gray-500"
                    >
                      User
                    </span>
                  {/if}
                </td>
                <td class="px-4 py-3">
                  <div class="flex flex-col gap-0.5">
                    <span
                      class="whitespace-nowrap text-[11px] {user.judge_runs_this_month >= user.judge_run_limit_effective
                        ? 'font-semibold text-brand-red'
                        : 'text-brand-muted'}"
                      title="Judge runs used this month / monthly limit{user.judge_run_limit != null ? ' (per-user override)' : ''}"
                    >
                      {user.judge_runs_this_month} / {user.judge_run_limit_effective} runs{user.judge_run_limit != null ? ' *' : ''}{user.judge_run_credits > 0 ? ` +${user.judge_run_credits}` : ''}
                    </span>
                  </div>
                </td>
                <td class="whitespace-nowrap px-4 py-3 text-sm text-brand-muted">{formatDate(user.created_at)}</td>
                <td class="px-4 py-3 text-right">
                  <div class="flex items-center justify-end gap-1">
                    <button
                      type="button"
                      onclick={() => openEditUser(user)}
                      class="rounded px-2.5 py-1 text-xs font-semibold text-brand-blue
                             transition-colors hover:bg-brand-blue/10"
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      onclick={() => openDeleteUser(user)}
                      class="rounded px-2.5 py-1 text-xs font-semibold text-red-500
                             transition-colors hover:bg-red-50"
                    >
                      Delete
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>

<UserFormModal
  open={userFormOpen}
  user={editingUser}
  onClose={() => (userFormOpen = false)}
/>

<Modal
  open={deleteTargetUser !== null}
  onClose={() => { deleteTargetUser = null; deleteError = undefined; }}
>
  <h2 class="mb-4 text-center font-heading text-2xl font-bold text-brand-text">Delete User</h2>
  <p class="mb-6 text-center text-sm text-brand-muted">
    Are you sure you want to delete
    <strong class="text-brand-text">{deleteTargetUser?.display_name}</strong>?
    This action cannot be undone.
  </p>

  {#if deleteError}
    <p class="mb-4 text-center text-sm text-brand-red">{deleteError}</p>
  {/if}

  <div class="flex gap-3">
    <button
      type="button"
      onclick={() => { deleteTargetUser = null; deleteError = undefined; }}
      class="flex-1 rounded-btn border-2 border-brand-border px-5 py-[10px]
             font-heading text-base font-bold text-brand-text
             transition-colors hover:border-brand-text"
    >
      Cancel
    </button>
    <button
      type="button"
      disabled={deleting}
      onclick={handleDeleteConfirm}
      class="flex-1 rounded-btn border-2 border-red-600 bg-red-600 px-5 py-[10px]
             font-heading text-base font-bold text-white
             transition-colors hover:bg-transparent hover:text-red-600 disabled:opacity-50"
    >
      {deleting ? 'Deleting…' : 'Delete'}
    </button>
  </div>
</Modal>