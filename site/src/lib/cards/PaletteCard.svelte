<script lang="ts">
	import { asset } from '$app/paths';
	import { suggestions } from '#lib/demo/script.js';
	import { envelope, span } from '#lib/ease.js';
	import { loop } from '#lib/motion.svelte.js';
	import Fit from './Fit.svelte';

	const QUERY = 'NVDA';
	const KEYS = [600, 760, 920, 1080];
	const MOVES = [2500, 3000];
	const ENTER = 3700;

	const RECENT = [
		{ code: 'WEI', title: 'World markets' },
		{ code: 'CONG', title: 'Congress trades' },
		{ code: 'GC', title: 'Treasury yield curve' },
		{ code: 'F7', title: 'Portfolio' }
	];

	const clock = loop(7200, 2000);
	let t = $derived(clock.t);

	let shown = $derived(envelope(t, 0, 250, 5800, 6400));
	let query = $derived(QUERY.slice(0, KEYS.filter((k) => t >= k).length));
	let rows = $derived(query ? suggestions(query) : RECENT);
	let selected = $derived(query ? MOVES.filter((m) => t >= m).length : -1);
	let pressed = $derived(t >= ENTER && t < ENTER + 160);
	let opened = $derived(span(t, ENTER, ENTER + 400));
	let caret = $derived(Math.floor(t / 530) % 2 === 0 || (t > KEYS[0] && t < KEYS[3] + 300));
</script>

<div class="card-visual" {@attach clock.observe}>
	<Fit width={506} height={304}>
		<img class="backdrop" src={asset('demo-dark.webp')} alt="" />
		<div class="palette" style:opacity={shown} style:transform="translateY({(1 - shown) * 8}px) scale({1 - opened * 0.02})">
			<div class="input">
				<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.6" /><path d="M10.4 10.4 14 14" /></svg>
				<span class="text">
					{#if query}
						{query}<span class="caret" style:opacity={caret ? 1 : 0}></span>
					{:else}
						<span class="caret" style:opacity={caret ? 1 : 0}></span><span class="placeholder">Type a ticker or command</span>
					{/if}
				</span>
			</div>
			{#each rows as row, i (row.code)}
				<div class={['row', { selected: i === selected, pressed: pressed && i === selected }]}>
					<span class="code">{row.code}</span>
					<span class="title">{row.title}</span>
					{#if 'kind' in row && row.kind}
						<span class="kind">{row.kind}</span>
					{/if}
					{#if i === selected}
						<span class="enter">↩</span>
					{/if}
				</div>
			{/each}
		</div>
	</Fit>
</div>

<style>
	.backdrop {
		position: absolute;
		inset: -12px;
		width: calc(100% + 24px);
		height: calc(100% + 24px);
		object-fit: cover;
		object-position: 20% 8%;
		filter: blur(3px) brightness(0.42) saturate(1.2);
	}

	.palette {
		position: absolute;
		left: 52px;
		top: 38px;
		width: 402px;
		padding: 6px;
		border-radius: 16px;
		background: rgba(28, 28, 32, 0.6);
		backdrop-filter: blur(18px) saturate(180%);
		box-shadow:
			inset 0 0.5px 0 rgba(255, 255, 255, 0.3),
			inset 0 0 0 0.5px rgba(255, 255, 255, 0.14),
			0 24px 60px -12px rgba(0, 0, 0, 0.7);
		transform-origin: 50% 30%;
	}

	.input {
		display: flex;
		align-items: center;
		gap: 10px;
		height: 44px;
		padding: 0 10px 4px;
		color: #f4f4f5;
		font: 500 18px var(--sans);
	}

	.input svg {
		width: 16px;
		height: 16px;
		fill: none;
		stroke: #8e8e93;
		stroke-width: 1.5;
		stroke-linecap: round;
	}

	.placeholder {
		color: #6e6e73;
	}

	.text {
		display: flex;
		align-items: center;
	}

	.caret {
		width: 1.5px;
		height: 20px;
		margin: 0 1px;
		background: #f4f4f5;
	}

	.row {
		display: flex;
		align-items: center;
		gap: 12px;
		height: 42px;
		padding: 0 16px;
		border-radius: 10px;
		color: #d1d1d6;
		font: 14px var(--sans);
		transition: background-color 0.18s;
	}

	.row.selected {
		background: rgba(255, 255, 255, 0.1);
		box-shadow: inset 0 0.5px 0 rgba(255, 255, 255, 0.18);
		color: #f4f4f5;
	}

	.row.pressed {
		background: rgba(255, 255, 255, 0.2);
	}

	.code {
		width: 38px;
		color: #8e8e93;
		font: 600 11px var(--mono);
	}

	.selected .code {
		color: #f4f4f5;
	}

	.title {
		flex: 1;
	}

	.kind {
		color: #8e8e93;
		font-size: 12px;
	}

	.enter {
		color: #8e8e93;
		font-size: 13px;
	}
</style>
