# Frontend Code Style

## TypeScript
- **Strict mode enabled**: All TypeScript must pass strict type checking
- Use explicit type annotations for function parameters/returns when not inferrable
- Prefer interfaces over types for object shapes; use `type` for unions/intersections

```typescript
// Good - explicit types
function loadModelAutomatically(configuration: Configuration, model: Model): boolean { }

// Good - interface for objects
interface Model { id: number; name: string; blob: Blob; group?: ModelGroup; }

// Good - type for unions
type FileType = 'stl' | 'obj' | '3mf' | 'step' | 'gcode';
```

## Svelte 5
- Use runes (`$state`, `$derived`, `$effect`) for reactive state
- Prefer `.svelte` files for components, `.ts` files for logic

```svelte
<script lang="ts">
    let count = $state(0);
    let doubled = $derived(count * 2);
</script>
<button onclick={() => count++}>{doubled}</button>
```

## Imports & Path Aliases
Defined in `svelte.config.js`:
```typescript
import { cn } from '$lib/utils';           // $lib -> ./src/lib
import { Model } from '$lib/api/shared/model_api';
```

## Naming Conventions
- **Files**: kebab-case (`model-api.ts`, `three-d-scene.svelte`)
- **Components**: PascalCase (`ModelGrid.svelte`, `Button.svelte`)
- **Types/Interfaces**: PascalCase (`Model`, `Configuration`)
- **Functions**: camelCase (`loadModelAutomatically`)
- **Constants**: SCREAMING_SNAKE_CASE for config, camelCase for others
- **CSS Classes**: kebab-case (Tailwind standard)

## Error Handling
Use try/catch for async, throw descriptive errors:
```typescript
try {
    const response = await fetch('/api/models');
    if (!response.ok) throw new Error(`Failed to fetch: ${response.status}`);
    return await response.json();
} catch (error) {
    console.error('Failed to fetch models:', error);
    throw error;
}
```

## TailwindCSS
Use utility classes; use `cn()` from `$lib/utils` for conditional classes:
```typescript
import { cn } from '$lib/utils';
<div class={cn("base-class", isActive && "active-class")}>
```
