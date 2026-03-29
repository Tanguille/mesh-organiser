<script lang="ts">
	import { writable } from 'svelte/store';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Picker } from '$lib/components/ui/picker/index.js';
	import { Slider, SliderThumb, SliderTrack, SliderFill, SliderTick, SliderMarker } from '$lib/components/ui/slider/index.js';
	import { Checkbox } from '$lib/components/ui/checkbox/index.js';
	
	export let modelId: number;
	export let onSlice: (settings: any) => void;
	
	// Default slicing settings
	const defaultSettings = writable({
		layerHeight: 0.2, // mm
		infill: 20, // percentage
		supports: 'none', // none, everywhere, touching buildplate
		material: 'PLA' // PLA, PETG, ABS, etc.
	});
	
	let settings = defaultSettings;
	
	function handleSlice() {
		onSlice(settings);
	}
	
	// Layer height options (in mm)
	const layerHeightOptions = [
		{ value: 0.1, label: '0.1 mm' },
		{ value: 0.2, label: '0.2 mm' },
		{ value: 0.3, label: '0.3 mm' }
	];
	
	// Infill percentage options (0-100)
	const infillOptions = [
		{ value: 0, label: '0%' },
		{ value: 5, label: '5%' },
		{ value: 10, label: '10%' },
		{ value: 15, label: '15%' },
		{ value: 20, label: '20%' },
		{ value: 25, label: '25%' },
		{ value: 30, label: '30%' },
		{ value: 35, label: '35%' },
		{ value: 40, label: '40%' },
		{ value: 45, label: '45%' },
		{ value: 50, label: '50%' },
		{ value: 55, label: '55%' },
		{ value: 60, label: '60%' },
		{ value: 65, label: '65%' },
		{ value: 70, label: '70%' },
		{ value: 75, label: '75%' },
		{ value: 80, label: '80%' },
		{ value: 85, label: '85%' },
		{ value: 90, label: '90%' },
		{ value: 95, label: '95%' },
		{ value: 100, label: '100%' }
	];
	
	// Support options
	const supportOptions = [
		{ value: 'none', label: 'None' },
		{ value: 'everywhere', label: 'Everywhere' },
		{ value: 'touching buildplate', label: 'Touching buildplate' }
	];
	
	// Material options
	const materialOptions = [
		{ value: 'PLA', label: 'PLA' },
		{ value: 'PETG', label: 'PETG' },
		{ value: 'ABS', label: 'ABS' },
		{ value: 'TPU', label: 'TPU' },
		{ value: 'NYLON', label: 'Nylon' },
		{ value: 'WOOD', label: 'Wood' },
		{ value: 'METAL', label: 'Metal' },
		{ value: 'CARBON_FIBER', label: 'Carbon Fiber' }
	];
</script>

<div class="space-y-6">
	<div>
		<label class="block text-sm font-medium mb-2">Layer Height</label>
		<Picker bind:value={settings.layerHeight} options={layerHeightOptions} />
	</div>
	
	<div>
		<label class="block text-sm font-medium mb-2">Infill Percentage</label>
		<Slider bind:value={settings.infill} min={0} max={100} step={1}>
			<SliderTrack />
			<SliderFill />
			<SliderTick ticks={11} />
			<SliderThumb />
		</Slider>
		<div class="flex justify-between text-xs text-muted mt-1">
			<span>0%</span>
			<span>100%</span>
		</div>
		<div class="text-center text-sm font-medium mt-1">{settings.infill}%</div>
	</div>
	
	<div>
		<label class="block text-sm font-medium mb-2">Supports</label>
		<Picker bind:value={settings.supports} options={supportOptions} />
	</div>
	
	<div>
		<label class="block text-sm font-medium mb-2">Material</label>
		<Picker bind:value={settings.material} options={materialOptions} />
	</div>
	
	<div class="pt-4">
		<Button on:click={handleSlice} class="w-full">
			Slice Model
		</Button>
	</div>
</div>

<style>
	/* Add any component-specific styles here */
</style>