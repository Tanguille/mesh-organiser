<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { derived } from 'svelte/store';
	import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
	import { SlicingSettings } from '$lib/components/SlicingSettings.svelte';
	import { Spinner } from '$lib/components/view/spinner.svelte';
	import { toast } from 'svelte-sonner';
	
	let model = null;
	let loading = true;
	let error = null;
	const modelId = $page.params.id;
	
	let slicedUrl = null;
	let slicing = false;
	let sliceError = null;
	
	async function loadModel() {
		try {
			loading = true;
			model = await meshOrganiserApi.getModel(Number(modelId));
			loading = false;
		} catch (err) {
			error = err;
			loading = false;
			console.error('Failed to load model:', err);
			toast.error('Failed to load model');
		}
	}
	
	async function handleSlice(settings) {
		try {
			slicing = true;
			sliceError = null;
			const result = await meshOrganiserApi.sliceModel(modelId, settings);
			if (result.success) {
				slicedUrl = result.slicedFileUrl;
				toast.success('Model sliced successfully');
			} else {
				sliceError = 'Slicing failed';
				toast.error('Slicing failed');
			}
		} catch (err) {
			sliceError = err.message || 'An error occurred during slicing';
			console.error('Slicing error:', err);
			toast.error('Slicing failed');
		} finally {
			slicing = false;
		}
	}
	
	function printModel() {
		if (slicedUrl) {
			goto(`/models/${modelId}/print/${encodeURIComponent(slicedUrl)}`);
		}
	}
	
	onMount(loadModel);
</script>

<div class="space-y-6">
	{#if loading}
		<div class="flex items-center justify-center py-12">
			<Spinner />
			<p class="ml-4">Loading model...</p>
		</div>
	{:else if error}
		<div class="text-center py-12">
			<p class="text-error">Error loading model: {error.message}</p>
			<button on:click={loadModel} class="mt-4 btn btn-primary">
				Retry
			</button>
		</div>
	{:else if model}
		<div>
			<h2 class="text-xl font-bold mb-4">{model.name}</h2>
			
			<div class="space-y-4">
				<h3 class="text-lg font-medium mb-2">Slicing Settings</h3>
				<SlicingSettings 
					{modelId} 
					onSlice={handleSlice} 
				/>
			</div>
			
			{#if slicing}
				<div class="flex items-center justify-center py-8">
					<Spinner />
					<p class="ml-4">Slicing model...</p>
				</div>
			{:else if sliceError}
				<div class="text-center py-8">
					<p class="text-error">{sliceError}</p>
				</div>
			{:else if slicedUrl}
				<div class="text-center py-8">
					<p class="text-success">Model sliced successfully!</p>
					<button 
						on:click={printModel} 
						class="mt-4 btn btn-primary"
					>
						Send to Printer
					</button>
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	/* Add any page-specific styles here */
</style>