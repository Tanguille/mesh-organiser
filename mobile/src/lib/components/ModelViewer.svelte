<script lang="ts">
	import { onMount } from 'svelte';
	import { onDestroy } from 'svelte';
	import { getContainer } from '$lib/api/dependency_injection';
	import type { Model } from '$lib/shared/model_api';
	import type { Blob } from '$lib/shared/blob_api';
	import { configuration } from '$lib/configuration.svelte';
	import { loadModel } from '$lib/workers/parseModelWorker';
	import { untrack } from 'svelte';
	import Spinner from '$lib/components/view/spinner.svelte';
	
	export let modelUri: string; // URI to the model data
	
	let geometry: any = null;
	let loading = true;
	let error: string | null = null;
	let loadGeneration = 0;
	let lastSuccessfulGeometryKey = "";
	let pendingGeometryKey = "";
	
	// For mobile, we'll use a simpler approach - just display a placeholder or basic visualization
	// In a real implementation, we might use a lightweight 3D library or just show thumbnails
	
	const blobId = modelUri.split('/').pop() || '';
	
	async function loadModelData() {
		if (!modelUri) return;
		
		loading = true;
		error = null;
		let localGeometry = null;
		
		try {
			// In a real implementation, we would fetch the model data from the URI
            // For now, we'll simulate loading or use a placeholder
            // Since we're getting a URI, we'd need to fetch the actual blob data
            
            // For mobile, let's keep it simple and show a placeholder
            // A more advanced implementation would fetch and parse the model
            
            // Simulate loading delay
            await new Promise(resolve => setTimeout(resolve, 1000));
            
            // In a real app, we would:
            // 1. Fetch the model data from modelUri
            // 2. Parse it using parseModelWorker
            // 3. Set the geometry
            
            // For now, we'll just indicate loading success without actual 3D rendering
            loading = false;
		} catch (err) {
			if (generation === loadGeneration) {
				console.warn("Failed to load model:", err);
				error = err instanceof Error ? err.message : "Failed to load model";
				loading = false;
			}
		} finally {
			if (generation === loadGeneration) {
				pendingGeometryKey = "";
			}
		}
	}
	
	function modelLoadKey(uri: string): string {
		return uri;
	}
	
	$effect(() => {
		if (!modelUri) return;
		
		const key = modelLoadKey(modelUri);
		if (key === lastSuccessfulGeometryKey && geometry) {
			return;
		}
		if (pendingGeometryKey === key) {
			return;
		}
		pendingGeometryKey = key;
		const gen = ++loadGeneration;
		untrack(() => loadModelData()));
	});
	
	onMount(() => {
		// Load model when component mounts
		if (modelUri) {
			loadModelData();
		}
	});
	
	onDestroy(() => {
		// Cleanup if needed
	});
</script>

<div class="model-viewer-container">
	{#if loading}
		<div class="flex items-center justify-center py-8">
			<span class="text-xl">Loading model...</span>
			<Spinner />
		</div>
	{:else if error}
		<div class="flex items-center justify-center py-8">
			<span class="text-xl text-red-600">Failed to load model</span>
			<span class="text-sm text-muted mt-2">{error}</span>
		</div>
	{:else}
		<!-- In a real implementation, we would render the 3D model here -->
		<!-- For mobile, we'll show a placeholder indicating 3D preview -->
		<div class="flex items-center justify-center py-8 text-center">
			<div class="w-24 h-24 bg-blue-100 rounded-lg flex items-center justify-center mb-4">
				<svg class="h-6 w-6 text-blue-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5a2 2 0 012-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
				</svg>
			</div>
			<p class="text-lg font-medium">3D Model Preview</p>
			<p class="text-sm text-muted">Interactive 3D viewer would be displayed here</p>
		</div>
	{/if}
</div>

<style>
	.model-viewer-container {
		width: 100%;
		height: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		text-align: center;
		padding: 1rem;
		background-color: var(--background-muted);
		border-radius: 0.5rem;
	}
</style>