<script lang="ts">
	import { goto } from '$app/navigation';
	import { writable } from 'svelte/store';
	import { Home } from "lucide-svelte";
	import { Upload } from "lucide-svelte";
	import { Printer } from "lucide-svelte";
	import { List } from "lucide-svelte";
	
	// Current tab store
	const currentTab = writable<'library' | 'import' | 'slice' | 'print'>('library');
	
	// Tab change functions
	function goToLibrary() {
		currentTab.set('library');
		goto('/');
	}
	
	function goToImport() {
		currentTab.set('import');
		goto('/import');
	}
	
	function goToSlice() {
		currentTab.set('slice');
		goto('/slice');
	}
	
	function goToPrint() {
		currentTab.set('print');
		goto('/print');
	}
	
	// Derived for template use
	let tab = $currentTab;
</script>

<style>
	.bottom-nav {
		position: fixed;
		bottom: 0;
		left: 0;
		right: 0;
		height: 60px;
		background-color: var(--background);
		border-top: 1px solid var(--border);
		display: flex;
		justify-content: space-around;
		align-items: center;
		padding: 0 1rem;
		z-index: 1000;
	}
	
	.nav-item {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 4px;
		cursor: pointer;
		padding: 8px;
		border-radius: 8px;
		transition: all 0.2s ease;
	}
	
	.nav-item:hover {
		background-color: var(--background-muted);
	}
	
	.nav-item.active {
		color: var(--primary);
		background-color: var(--background-muted);
	}
	
	.nav-icon {
		width: 24px;
		height: 24px;
	}
	
	.nav-label {
		font-size: 12px;
		font-weight: 500;
	}
</style>

<div class="bottom-nav">
	<div 
		class={`nav-item ${tab === 'library' ? 'active' : ''}`}
		on:click={goToLibrary}
	>
		<Home class="nav-icon" />
		<div class="nav-label">Library</div>
	</div>
	
	<div 
		class={`nav-item ${tab === 'import' ? 'active' : ''}`}
		on:click={goToImport}
	>
		<Upload class="nav-icon" />
		<div class="nav-label">Import</div>
	</div>
	
	<div 
		class={`nav-item ${tab === 'slice' ? 'active' : ''}`}
		on:click={goToSlice}
	>
		<Printer class="nav-icon" />
		<div class="nav-label">Slice</div>
	</div>
	
	<div 
		class={`nav-item ${tab === 'print' ? 'active' : ''}`}
		on:click={goToPrint}
	>
		<List class="nav-icon" />
		<div class="nav-label">Print</div>
	</div>
</div>