<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';
	import { Spinner } from '$lib/components/view/spinner.svelte';
	import { toast } from 'svelte-sonner';
	
	let model = null;
	let loading = true;
	let error = null;
	const modelId = $page.params.id;
	const slicedUrl = decodeURIComponent($page.params.url);
	
	let printers = [];
	let loadingPrinters = true;
	let errorPrinters = null;
	let selectedPrinterId = null;
	let printJobId = null;
	let printStatus = null;
	let printProgress = 0;
	let printError = null;
	let pollingInterval = null;
	
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
	
	async function loadPrinters() {
		try {
			loadingPrinters = true;
			printers = await meshOrganiserApi.getPrinters();
			loadingPrinters = false;
		} catch (err) {
			errorPrinters = err;
			loadingPrinters = false;
			console.error('Failed to load printers:', err);
			toast.error('Failed to load printers');
		}
	}
	
	async function startPrint() {
		if (!selectedPrinterId) return;
		
		try {
			const result = await meshOrganiserApi.startPrint(selectedPrinterId, modelId);
			printJobId = result.id;
			
			// Start polling for print status
			pollingInterval = setInterval(async () => {
				try {
					const status = await meshOrganiserApi.getPrintStatus(printJobId);
					printStatus = status.status;
					printProgress = status.progress;
					
					if (['completed', 'failed', 'cancelled'].includes(status.status)) {
						clearInterval(pollingInterval);
						pollingInterval = null;
						
						if (status.status === 'failed') {
							printError = status.error || 'Print failed';
							toast.error('Print failed');
						} else if (status.status === 'completed') {
							toast.success('Print completed successfully');
						}
					}
				} catch (err) {
					console.error('Error getting print status:', err);
					clearInterval(pollingInterval);
					pollingInterval = null;
					printError = 'Failed to get print status';
					toast.error('Failed to get print status');
				}
			}, 5000); // Poll every 5 seconds
			
		} catch (err) {
			printError = err.message || 'Failed to start print';
			console.error('Print start error:', err);
			toast.error('Failed to start print');
		}
	}
	
	function cancelPrint() {
		if (printJobId && pollingInterval) {
			clearInterval(pollingInterval);
			pollingInterval = null;
			// TODO: Implement actual print cancellation via API
			toast.info('Print cancelled');
			goto(`/`);
		}
	}
	
	function pausePrint() {
		// TODO: Implement print pause via API
		toast.info('Print paused');
	}
	
	function resumePrint() {
		// TODO: Implement print resume via API
		toast.info('Print resumed');
	}
	
	onMount(async () => {
		await loadModel();
		await loadPrinters();
	});
	
	// Clean up interval on destroy
	// onDestroy(() => {
	//   if (pollingInterval) {
	//     clearInterval(pollingInterval);
	//   }
	// });
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
				<h3 class="text-lg font-medium mb-2">Select Printer</h3>
				
				{#if loadingPrinters}
					<div class="flex items-center justify-center py-8">
						<Spinner />
						<p class="ml-4">Loading printers...</p>
					</div>
				{:else if errorPrinters}
					<div class="text-center py-8">
						<p class="text-error">Error loading printers: {errorPrinters.message}</p>
					</div>
				{:else if printers.length === 0}
					<div class="text-center py-8">
						<p class="text-warning">No printers configured</p>
						<p class="text-sm text-muted">Please configure printers in the web interface</p>
					</div>
				{:else}
					<div class="space-y-4">
						{#each printers as printer}
							<div class="border border-gray-200 rounded-lg p-4">
								<div class="flex justify-between items-start">
									<div>
										<h4 class="font-medium">{printer.name}</h4>
										<p class="text-sm text-muted">
											Status: 
											<span class="px-2 py-0.5 rounded 
												{#if printer.status === 'idle'}bg-green-100 text-green-800
												{#else if printer.status === 'printing'}bg-blue-100 text-blue-800
												{#else if printer.status === 'paused'}bg-yellow-100 text-yellow-800
												{#else}bg-red-100 text-red-800{/if}
											">
												{printer.status}
											</span>
										</p>
									</div>
									<div class="flex-shrink-0">
										<button 
											on:click={() => selectedPrinterId = printer.id}
											class="btn btn-outline 
												{#if selectedPrinterId === printer.id}btn-primary{else}btn-outline{/if}
											"
										>
											{#if selectedPrinterId === printer.id}
												Selected
											{else}
												Select
											{/if}
										</button>
									</div>
								</div>
							</div>
						{/each}
					</div>
				{/if}
				
				<div class="mt-6">
					<button 
						on:click={startPrint}
						class="btn btn-primary w-full"
						disabled={!selectedPrinterId || loadingPrinters}
					>
						{#if loadingPrinters}
							Starting...
						{:else if !selectedPrinterId}
							Select a printer to start
						{:else}
							Start Print
						{/if}
					</button>
				</div>
			</div>
			
			{#if printJobId}
				<div class="border border-gray-200 rounded-lg p-4">
					<h3 class="text-lg font-medium mb-4">Print Job Status</h3>
					
					<div class="space-y-4">
						<div class="flex items-center">
							<div class="w-12 h-12">
								{#if printStatus === 'printing'}
									<Spinner class="h-8 w-8" />
								{:else if printStatus === 'completed'}
									<svg class="h-6 w-6 text-green-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
										<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
									</svg>
								{:else if printStatus === 'failed'}
									<svg class="h-6 w-6 text-red-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
										<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
									</svg>
								{:else}
									<svg class="h-6 w-6 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
										<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l4 4" />
									</svg>
								{/if}
							</div>
							<div class="ml-4">
								<p class="font-medium">Status: {printStatus}</p>
								<p class="text-sm text-muted">Progress: {printProgress}%</p>
							</div>
						</div>
						
						<div class="progress w-full bg-gray-200 rounded-full h-2.5">
							<div class="progress-bar bg-blue-600 h-2.5 rounded-full" style="width: {printProgress}%"></div>
						</div>
						
						<div class="flex justify-between items-center mt-4">
							<button 
								on:click={pausePrint}
								class="btn btn-outline mr-2"
								disabled={printStatus !== 'printing'}
							>
								Pause
							</button>
							<button 
								on:click={resumePrint}
								class="btn btn-outline mr-2"
								disabled={printStatus !== 'paused'}
							>
								Resume
							</button>
							<button 
								on:click={cancelPrint}
								class="btn btn-error"
							>
								Cancel Print
							</button>
						</div>
						
						{#if printError}
							<div class="mt-4 p-4 bg-red-50 rounded border border-red-200">
								<p class="text-red-600">Error: {printError}</p>
							</div>
						{/if}
					</div>
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	/* Add any page-specific styles here */
</style>