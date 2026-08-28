<script lang="ts">
	import { marked } from 'marked';
	import DOMPurify from 'isomorphic-dompurify';

	// Add rel="noopener noreferrer" to all target="_blank" links once at module load.
	DOMPurify.addHook('afterSanitizeAttributes', (node) => {
		if (node.tagName === 'A' && node.getAttribute('target') === '_blank') {
			node.setAttribute('rel', 'noopener noreferrer');
		}
	});

	let { value = '', class: extraClass = '', style: extraStyle = '', breaks = false }: { value: string; class?: string; style?: string; breaks?: boolean } = $props();

	const html = $derived(
		DOMPurify.sanitize(marked.parse(value, { async: false, breaks }) as string, { USE_PROFILES: { html: true } })
	);
</script>

<div class="prose prose-sm max-w-none {extraClass}" style={extraStyle}>{@html html}</div>
