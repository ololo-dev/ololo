<script lang="ts">
	import { X, CheckCircle2, XCircle, AlertTriangle, Info, ChevronDown, ChevronUp } from 'lucide-svelte';
	import { notify, type NotificationType } from '$lib/notifications.svelte';

	const MAX_VISIBLE = 3;

	let expanded = $state(false);

	const icons: Record<NotificationType, typeof CheckCircle2> = {
		success: CheckCircle2,
		error: XCircle,
		warning: AlertTriangle,
		info: Info
	};

	// Colors aligned with the app's brand palette (tailwind.config.ts)
	// success → brand-blue (primary positive action)
	// error   → brand-red
	// warning → brand-muted (no dedicated warning hue in the palette)
	// info    → brand-check-blue / brand-light-blue
	const circleBg: Record<NotificationType, string> = {
		success: 'bg-brand-tag-bg',
		error: 'bg-brand-red/10',
		warning: 'bg-brand-border',
		info: 'bg-brand-light-blue'
	};

	const iconColor: Record<NotificationType, string> = {
		success: 'text-brand-blue',
		error: 'text-brand-red',
		warning: 'text-brand-muted',
		info: 'text-brand-check-blue'
	};

	const progressColor: Record<NotificationType, string> = {
		success: 'bg-brand-blue',
		error: 'bg-brand-red',
		warning: 'bg-brand-muted',
		info: 'bg-brand-check-blue'
	};

	const visible = $derived(
		expanded ? [...notify.items].reverse() : [...notify.items].reverse().slice(0, MAX_VISIBLE)
	);

	const hidden = $derived(Math.max(0, notify.items.length - MAX_VISIBLE));

	$effect(() => {
		if (notify.items.length === 0) expanded = false;
	});
</script>

<!-- Fixed overlay — SSR-safe, no browser API at mount -->
<div
	role="region"
	aria-live="polite"
	aria-label="Notifications"
	class="pointer-events-none fixed right-4 top-4 z-50 flex w-80 flex-col gap-2 sm:right-6 sm:top-6 sm:w-96"
>
	{#if notify.items.length > 0}
		<!-- Header: total count + clear all -->
		<div
			class="pointer-events-auto flex items-center justify-between rounded-xl border border-brand-border bg-white px-3 py-1.5 shadow-sm"
		>
			<span class="flex items-center gap-2 text-xs font-medium text-brand-muted">
				<span
					class="inline-flex h-5 min-w-[1.25rem] items-center justify-center rounded-full bg-brand-blue px-1.5 text-[10px] font-bold text-white"
				>
					{notify.items.length}
				</span>
				{notify.items.length === 1 ? 'notification' : 'notifications'}
			</span>
			<button
				onclick={() => notify.dismissAll()}
				class="font-heading text-xs text-brand-muted underline-offset-2 transition-colors hover:text-brand-text hover:underline"
			>
				Clear all
			</button>
		</div>

		<!-- Cards -->
		{#each visible as notification (notification.id)}
			{@const Icon = icons[notification.type]}
			<div
				class="pointer-events-auto relative overflow-hidden rounded-2xl bg-white shadow-lg"
				role="alert"
			>
				<!-- Content row -->
				<div class="flex items-center gap-4 px-5 py-4 pr-10">
					<!-- Icon circle -->
					<div
						class="flex h-14 w-14 shrink-0 items-center justify-center rounded-full {circleBg[notification.type]}"
					>
						<Icon class="h-7 w-7 {iconColor[notification.type]}" />
					</div>

					<!-- Text -->
					<div class="min-w-0 flex-1">
						{#if notification.title}
							<p class="font-heading text-sm font-semibold text-brand-text">{notification.title}</p>
						{/if}
						<p class="text-sm text-brand-text {notification.title ? 'mt-0.5 opacity-80' : ''}">
							{notification.message}
						</p>
					</div>
				</div>

				<!-- Dismiss button -->
				<button
					onclick={() => notify.dismiss(notification.id)}
					class="absolute right-3 top-3 rounded p-1 text-brand-muted transition-colors hover:text-brand-text focus-visible:outline focus-visible:outline-2"
					aria-label="Dismiss notification"
				>
					<X class="h-4 w-4" />
				</button>

				<!-- Countdown bar — shrinks from full to empty over `duration` ms -->
				{#if notification.duration > 0}
					<div class="h-0.5 w-full">
						<div
							class="countdown-bar h-full {progressColor[notification.type]}"
							style="animation-duration: {notification.duration}ms"
						></div>
					</div>
				{/if}
			</div>
		{/each}

		<!-- Expand / collapse toggle -->
		{#if hidden > 0 || expanded}
			<button
				onclick={() => (expanded = !expanded)}
				class="pointer-events-auto flex items-center justify-center gap-1.5 rounded-xl border border-brand-border bg-white py-1.5 font-heading text-xs font-medium text-brand-muted shadow-sm transition-colors hover:bg-brand-light-blue hover:text-brand-text"
			>
				{#if expanded}
					<ChevronUp class="h-3.5 w-3.5" />
					Show fewer
				{:else}
					<ChevronDown class="h-3.5 w-3.5" />
					{hidden} more
				{/if}
			</button>
		{/if}
	{/if}
</div>

<style>
	.countdown-bar {
		animation-name: countdown;
		animation-timing-function: linear;
		animation-fill-mode: forwards;
	}

	@keyframes countdown {
		from {
			width: 100%;
		}
		to {
			width: 0%;
		}
	}
</style>
