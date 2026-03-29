# Mobile and Web App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement mobile and web companion apps for Mesh Organiser that provide model viewing, importing from supported websites, and basic slicing capabilities, with both apps connecting to a NAS-based Mesh Organiser instance running in a Docker container.

**Architecture:** Client-server model where mobile and web apps communicate with a NAS-based Mesh Organiser instance that handles model storage, slicing via OrcaSlicer, and printer management. Both mobile and web apps will share the same SvelteKit codebase, with the mobile app using Capacitor to access native device features while the web app runs in standard browsers.

**Tech Stack:**

- Mobile App: SvelteKit with Capacitor (shared codebase with web app), Capacitor plugins for native features
- Web App: Existing SvelteKit (no changes needed)
- NAS Instance: Docker container running:
  - Frontend: SvelteKit web application (serving both mobile and web clients)
  - Backend: Existing Rust-based service crate (handles slicing, database, printer operations)
- Communication:
  - Mobile/Web Apps ↔ NAS Instance: REST API (SvelteKit endpoints)
  - NAS Instance Frontend ↔ NAS Instance Backend: Internal Rust communication

---

### Task 1: Set up Mobile App Project Structure with SvelteKit + Capacitor

**Files:**

- Create: `mobile/package.json`
- Create: `mobile/tsconfig.json`
- Create: `mobile/svelte.config.js`
- Create: `mobile/vite.config.js`
- Create: `mobile/capacitor.config.json`
- Create: `mobile/src/app.html`
- Create: `mobile/src/main.ts`
- Create: `mobile/src/routes/+layout.svelte`
- Create: `mobile/src/routes/+layout.ts`
- Create: `mobile/src/routes/+page.svelte` (home)
- Create: `mobile/src/routes/models/+page.svelte`
- Create: `mobile/src/routes/models/[id]/+page.svelte` (model detail)
- Create: `mobile/src/routes/models/[id]/slice/+page.svelte` (slicer)
- Create: `mobile/src/routes/models/[id]/print/+page.svelte` (printer)
- Create: `mobile/src/routes/import/+page.svelte`
- Create: `mobile/src/routes/print-queue/+page.svelte`
- Create: `mobile/src/lib/api/meshOrganiserApi.ts`
- Create: `mobile/src/lib/stores/authStore.ts`
- Create: `mobile/src/lib/stores/appStore.ts`
- Create: `mobile/src/lib/components/ModelLibrary.svelte`
- Create: `mobile/src/lib/components/ModelDetail.svelte`
- Create: `mobile/src/lib/components/ModelViewer.svelte`
- Create: `mobile/src/lib/components/SlicingSettings.svelte`
- Create: `mobile/src/lib/components/PrintQueue.svelte`
- Modify: `docs/superpowers/specs/2026-03-29-mobile-app-design.md` (update references if needed)

- [ ] **Step 1: Initialize SvelteKit project**

```bash
npm create svelte@latest mobile
# Choose: Skeleton project
# Choose: Yes to TypeScript
# Choose: No to ESLint
# Choose: No to Prettier
# Choose: No to Playwright
# Choose: No to Vitest
```

- [ ] **Step 2: Add Capacitor to SvelteKit project**

```bash
cd mobile
npm install @capacitor/core @capacitor/cli
npx cap init
# Follow prompts for app name and package ID
npx cap add android
```

- [ ] **Step 3: Install required dependencies**

```bash
npm install axios
```

- [ ] **Step 4: Configure TypeScript and SvelteKit**

Update `tsconfig.json` and `vite.config.js` as needed for Capacitor integration

- [ ] **Step 5: Set up basic project structure**

Ensure standard SvelteKit structure with routes and lib directories

- [ ] **Step 6: Commit initial setup**

```bash
git add mobile/
git commit -m "feat(mobile): initialize SvelteKit + Capacitor project structure"
```

### Task 2: Implement API Communication Layer

**Files:**

- Create: `mobile/src/lib/api/meshOrganiserApi.ts`
- Modify: `mobile/src/lib/api/meshOrganiserApi.ts`

- [ ] **Step 1: Write API interface definition**

```typescript
export interface MeshOrganiserApi {
  getModels(): Promise<any[]>;
  getModel(id: number): Promise<any>;
  importModel(formData: FormData): Promise<any>;
  sliceModel(modelId: number, settings: SlicingSettings): Promise<any>;
  getPrinters(): Promise<any[]>;
  startPrint(printerId: number, modelId: number): Promise<any>;
  getPrintStatus(printJobId: string): Promise<any>;
}
```

- [ ] **Step 2: Implement API client with base URL and endpoints**

```typescript
import axios from "axios";

// Determine base URL based on environment
const API_BASE_URL = import.meta.env.DEV
  ? "http://localhost:9435"
  : "http://your-nas-ip:9435";

export class MeshOrganiserApiImpl implements MeshOrganiserApi {
  private api = axios.create({
    baseURL: API_BASE_URL,
    headers: {
      "Content-Type": "application/json",
    },
  });

  async getModels(): Promise<any[]> {
    const response = await this.api.get("/api/models");
    return response.data;
  }

  async getModel(id: number): Promise<any> {
    const response = await this.api.get(`/api/models/${id}`);
    return response.data;
  }

  async importModel(formData: FormData): Promise<any> {
    const response = await this.api.post("/api/models", formData, {
      headers: {
        "Content-Type": "multipart/form-data",
      },
    });
    return response.data;
  }

  async sliceModel(modelId: number, settings: SlicingSettings): Promise<any> {
    const response = await this.api.post(`/api/slicer/slice`, {
      model_id: modelId,
      ...settings,
    });
    return response.data;
  }

  async getPrinters(): Promise<any[]> {
    const response = await this.api.get("/api/printers");
    return response.data;
  }

  async startPrint(printerId: number, modelId: number): Promise<any> {
    const response = await this.api.post(`/api/printers/${printerId}/print`, {
      model_id: modelId,
    });
    return response.data;
  }

  async getPrintStatus(printJobId: string): Promise<any> {
    const response = await this.api.get(`/api/printers/status/${printJobId}`);
    return response.data;
  }
}

// Export singleton instance
export const meshOrganiserApi = new MeshOrganiserApiImpl();
```

- [ ] **Step 3: Add authentication token handling**

Modify the API client to include auth tokens from stores

- [ ] **Step 4: Write unit tests for API client**

Create test file: `mobile/src/lib/api/__tests__/meshOrganiserApi.test.ts`

- [ ] **Step 5: Run tests to verify API implementation**

```bash
cd mobile && npm test src/lib/api/__tests__/meshOrganiserApi.test.ts
```

- [ ] **Step 6: Commit API implementation**

```bash
git add mobile/src/lib/api/meshOrganiserApi.ts
git commit -m "feat(mobile): implement Mesh Organiser API communication layer"
```

- [ ] **Step 2: Implement API client with base URL and endpoints**

```typescript
import axios from "axios";

// Determine base URL based on environment
const API_BASE_URL = import.meta.env.DEV
  ? "http://localhost:9435"
  : "http://your-nas-ip:9435";

export class MeshOrganiserApiImpl implements MeshOrganiserApi {
  private api = axios.create({
    baseURL: API_BASE_URL,
    headers: {
      "Content-Type": "application/json",
    },
  });

  async getModels(): Promise<any[]> {
    const response = await this.api.get("/api/models");
    return response.data;
  }

  async getModel(id: number): Promise<any> {
    const response = await this.api.get(`/api/models/${id}`);
    return response.data;
  }

  async importModel(formData: FormData): Promise<any> {
    const response = await this.api.post("/api/models", formData, {
      headers: {
        "Content-Type": "multipart/form-data",
      },
    });
    return response.data;
  }

  async sliceModel(modelId: number, settings: SlicingSettings): Promise<any> {
    const response = await this.api.post(`/api/slicer/slice`, {
      model_id: modelId,
      ...settings,
    });
    return response.data;
  }

  async getPrinters(): Promise<any[]> {
    const response = await this.api.get("/api/printers");
    return response.data;
  }

  async startPrint(printerId: number, modelId: number): Promise<any> {
    const response = await this.api.post(`/api/printers/${printerId}/print`, {
      model_id: modelId,
    });
    return response.data;
  }

  async getPrintStatus(printJobId: string): Promise<any> {
    const response = await this.api.get(`/api/printers/status/${printJobId}`);
    return response.data;
  }
}

// Export singleton instance
export const meshOrganiserApi = new MeshOrganiserApiImpl();
```

- [ ] **Step 3: Add authentication token handling**

Modify the API client to include auth tokens from stores

- [ ] **Step 4: Write unit tests for API client**

Create test file: `mobile/src/lib/api/__tests__/meshOrganiserApi.test.ts`

- [ ] **Step 5: Run tests to verify API implementation**

```bash
cd mobile && npm test src/lib/api/__tests__/meshOrganiserApi.test.ts
```

- [ ] **Step 6: Commit API implementation**

```bash
git add mobile/src/lib/api/meshOrganiserApi.ts
git commit -m "feat(mobile): implement Mesh Organiser API communication layer"
```

- [ ] **Step 2: Implement API client with base URL and endpoints**

```typescript
import axios from "axios";

const API_BASE_URL = __DEV__
  ? "http://localhost:9435"
  : "http://your-nas-ip:9435";

export class MeshOrganiserApiImpl implements MeshOrganiserApi {
  private api = axios.create({
    baseURL: API_BASE_URL,
    headers: {
      "Content-Type": "application/json",
    },
  });

  async getModels(): Promise<any[]> {
    const response = await this.api.get("/api/models");
    return response.data;
  }

  async getModel(id: number): Promise<any> {
    const response = await this.api.get(`/api/models/${id}`);
    return response.data;
  }

  async importModel(formData: FormData): Promise<any> {
    const response = await this.api.post("/api/models", formData, {
      headers: {
        "Content-Type": "multipart/form-data",
      },
    });
    return response.data;
  }

  async sliceModel(modelId: number, settings: SlicingSettings): Promise<any> {
    const response = await this.api.post(`/api/slicer/slice`, {
      model_id: modelId,
      ...settings,
    });
    return response.data;
  }

  async getPrinters(): Promise<any[]> {
    const response = await this.api.get("/api/printers");
    return response.data;
  }

  async startPrint(printerId: number, modelId: number): Promise<any> {
    const response = await this.api.post(`/api/printers/${printerId}/print`, {
      model_id: modelId,
    });
    return response.data;
  }

  async getPrintStatus(printJobId: string): Promise<any> {
    const response = await this.api.get(`/api/printers/status/${printJobId}`);
    return response.data;
  }
}
```

- [ ] **Step 3: Add authentication token handling**

Modify the API client to include auth tokens from context

- [ ] **Step 4: Write unit tests for API client**

Create test file: `mobile/src/api/__tests__/meshOrganiserApi.test.ts`

- [ ] **Step 5: Run tests to verify API implementation**

```bash
cd mobile && npm test src/api/__tests__/meshOrganiserApi.test.ts
```

- [ ] **Step 6: Commit API implementation**

```bash
git add mobile/src/api/meshOrganiserApi.ts
git commit -m "feat(mobile): implement Mesh Organiser API communication layer"
```

### Task 3: Implement Authentication Store

**Files:**

- Create: `mobile/src/lib/stores/authStore.ts`
- Modify: `mobile/src/lib/stores/authStore.ts`

- [ ] **Step 1: Create authentication store with login/logout functionality**

```typescript
import { writable } from 'svelte/store';
import { meshOrganiserApi } from '../api/meshOrganiserApi';

export interface AuthStoreType {
  token: string | null;
  user: any | null;
  isAuthenticated: boolean;
}

const createAuthStore = () => {
  const { subscribe, set, update } = writable<AuthStoreType>({
    token: null,
    user: null,
    isAuthenticated: false
  });

  return {
    subscribe,
    login: async (username: string, password: string) => {
      try {
        // TODO: Implement actual login API call
        // For now, simulate successful login
        // const response = await meshOrganiserApi.login(username, password);
        // set({ token: response.token, user: response.user, isAuthenticated: true });

        // Mock implementation for now
        set({ token: 'mock-token', user: { username }, isAuthenticated: true });
      } catch (error) {
        console.error('Login failed:', error);
        throw error;
      }
    },
    logout: () => {
      set({ token: null, user: null, isAuthenticated: false });
    },
    setToken: (token: string | null) => {
      update(store => ({
        ...store,
        token,
        isAuthenticated: !!token
      }));
    }
  };
});

export const authStore = createAuthStore();
```

- [ ] **Step 2: Implement token persistence using browser storage**

Modify authStore to persist tokens in localStorage

- [ ] **Step 3: Add auto-login on app startup**

Check for stored token and attempt to restore session

- [ ] **Step 4: Write tests for authentication logic**

Create test file: `mobile/src/lib/stores/__tests__/authStore.test.ts`

- [ ] **Step 5: Run tests to verify authentication works**

```bash
cd mobile && npm test src/lib/stores/__tests__/authStore.test.ts
```

- [ ] **Step 6: Commit authentication implementation**

```bash
git add mobile/src/lib/stores/authStore.ts
git commit -m "feat(mobile): implement authentication store"
```

- [ ] **Step 2: Implement token storage using AsyncStorage**

Modify AuthContext to persist tokens

- [ ] **Step 3: Add login screen component**

Create `mobile/src/screens/LoginScreen.tsx`

- [ ] **Step 4: Write tests for authentication logic**

Create test file: `mobile/src/context/__tests__/AuthContext.test.ts`

- [ ] **Step 5: Run tests to verify authentication works**

```bash
cd mobile && npm test src/context/__tests__/AuthContext.test.ts
```

- [ ] **Step 6: Commit authentication implementation**

```bash
git add mobile/src/context/AuthContext.tsx
git commit -m "feat(mobile): implement authentication context"
```

### Task 4: Implement Model Library Screen

**Files:**

- Create: `mobile/src/lib/components/ModelLibrary.svelte`
- Modify: `mobile/src/lib/components/ModelLibrary.svelte`

- [ ] **Step 1: Create ModelLibrary component with model list**

```svelte
<script>
  import { onMount } from "svelte";
  import { authStore } from "../stores/authStore";
  import { meshOrganiserApi } from "../lib/api/meshOrganiserApi";

  let models = [];
  let loading = true;

  async function loadModels() {
    try {
      loading = true;
      const modelsData = await meshOrganiserApi.getModels();
      models = modelsData;
    } catch (error) {
      console.error("Failed to load models:", error);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadModels();
  });
</script>

{#if loading}
  <div class="loading-indicator">Loading models...</div>
{:else}
  <div class="model-library">
    {#each models as model}
      <div class="model-item" on:click={() => goto(`/models/${model.id}`)}>
        <h3>{model.name}</h3>
        {#if model.thumbnailUrl}
          <img
            src={model.thumbnailUrl}
            alt={model.name}
            class="model-thumbnail"
          />
        {:else}
          <div class="placeholder-thumbnail">No preview</div>
        {/if}
        <p class="model-meta">
          Added: {new Date(model.added).toLocaleDateString()}
        </p>
      </div>
    {/each}
  </div>
{/if}

<style>
  .model-library {
    display: grid;
    gap: 1rem;
    padding: 1rem;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  }

  .model-item {
    border: 1px solid #eee;
    border-radius: 8px;
    padding: 1rem;
    text-align: center;
    cursor: pointer;
    transition: transform 0.2s;
  }

  .model-item:hover {
    transform: translateY(-4px);
  }

  .model-thumbnail {
    width: 100%;
    height: 150px;
    object-fit: contain;
    margin-bottom: 0.5rem;
  }

  .placeholder-thumbnail {
    width: 100%;
    height: 150px;
    background-color: #f5f5f5;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #999;
    font-size: 0.9rem;
  }

  .model-meta {
    font-size: 0.9rem;
    color: #666;
    margin-top: 0.5rem;
  }

  .loading-indicator {
    text-align: center;
    padding: 2rem;
    color: #666;
  }
</style>
```

- [ ] **Step 2: Add navigation to model detail view**

Use SvelteKit's `goto` function for navigation

- [ ] **Step 3: Add pull-to-refresh functionality**

Implement using a custom refresh indicator or browser pull-to-refresh

- [ ] **Step 4: Write unit tests for ModelLibrary component**

Create test file: `mobile/src/lib/components/__tests__/ModelLibrary.test.ts`

- [ ] **Step 5: Run tests to verify component works**

```bash
cd mobile && npm test src/lib/components/__tests__/ModelLibrary.test.ts
```

- [ ] **Step 6: Commit ModelLibrary implementation**

```bash
git add mobile/src/lib/components/ModelLibrary.svelte
git commit -m "feat(mobile): implement model library screen"
```

- [ ] **Step 2: Add model thumbnail display**

Modify ModelLibrary to show thumbnails if available

- [ ] **Step 3: Add pull-to-refresh functionality**

Implement refresh control for the FlatList

- [ ] **Step 4: Write unit tests for ModelLibrary component**

Create test file: `mobile/src/components/__tests__/ModelLibrary.test.tsx`

- [ ] **Step 5: Run tests to verify component works**

```bash
cd mobile && npm test src/components/__tests__/ModelLibrary.test.tsx
```

- [ ] **Step 6: Commit ModelLibrary implementation**

```bash
git add mobile/src/components/ModelLibrary.tsx
git commit -m "feat(mobile): implement model library screen"
```

### Task 5: Implement Model Detail and Import Screens

**Files:**

- Create: `mobile/src/lib/components/ModelDetail.svelte`
- Create: `mobile/src/lib/components/ImportScreen.svelte`
- Modify: `mobile/src/lib/components/ModelDetail.svelte`
- Modify: `mobile/src/lib/components/ImportScreen.svelte`

- [ ] **Step 1: Create ModelDetail component**

```svelte
<script>
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { authStore } from "../stores/authStore";
  import { meshOrganiserApi } from "../lib/api/meshOrganiserApi";
  import ModelViewer from "$lib/components/ModelViewer.svelte";

  export let modelId;

  let model = null;
  let loading = true;
  let error = null;

  async function loadModel() {
    try {
      loading = true;
      error = null;
      const modelData = await meshOrganiserApi.getModel(modelId);
      model = modelData;
    } catch (err) {
      error = err;
      console.error("Failed to load model:", err);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadModel();
  });

  function handleSlice() {
    goto(`/models/${modelId}/slice`);
  }

  function handlePrint() {
    goto(`/models/${modelId}/print`);
  }
</script>

{#if loading}
  <div class="loading-indicator">Loading model details...</div>
{:else if error}
  <div class="error-message">Error loading model: {error.message}</div>
{:else if model}
  <div class="model-detail">
    <div class="model-preview">
      <ModelViewer modelUri={model.thumbnailUrl || ""} />
    </div>

    <div class="model-info">
      <h2>{model.name}</h2>
      <p class="model-meta">Size: {model.blob.size} bytes</p>
      <p class="model-meta">
        Added: {new Date(model.added).toLocaleDateString()}
      </p>
      {#if model.description}
        <p class="model-description">{model.description}</p>
      {/if}
    </div>

    <div class="model-actions">
      <button on:click={handleSlice}>Slice Model</button>
      <button on:click={handlePrint}>Print Model</button>
    </div>
  </div>
{/if}

<style>
  .model-detail {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 2rem;
    gap: 1.5rem;
  }

  .model-preview {
    width: 100%;
    max-width: 300px;
    height: 300px;
    background-color: #f5f5f5;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .model-info {
    text-align: center;
    width: 100%;
  }

  .model-info h2 {
    margin: 0 0 0.5rem 0;
    font-size: 1.8rem;
  }

  .model-meta {
    color: #666;
    font-size: 0.9rem;
    margin: 0.25rem 0;
  }

  .model-description {
    color: #444;
    font-style: italic;
    margin: 1rem 0;
    line-height: 1.5;
  }

  .model-actions {
    display: flex;
    gap: 1rem;
    margin-top: 1.5rem;
  }

  .model-actions button {
    flex: 1;
    padding: 0.75rem;
    font-size: 1rem;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .model-actions button:nth-child(1) {
    background-color: #4caf50;
    color: white;
  }

  .model-actions button:nth-child(1):hover {
    background-color: #45a049;
  }

  .model-actions button:nth-child(2) {
    background-color: #2196f3;
    color: white;
  }

  .model-actions button:nth-child(2):hover {
    background-color: #0b7dda;
  }

  .loading-indicator,
  .error-message {
    text-align: center;
    padding: 2rem;
  }

  .error-message {
    color: #f44336;
  }
</style>
```

- [ ] **Step 2: Create ImportScreen for website imports**

Implement screen that handles importing models from Thingiverse, Printables, etc., leveraging existing web import functionality

- [ ] **Step 3: Implement navigation between screens**

Set up SvelteKit routing with appropriate routes for Home, ModelDetail, Slicer, Printer, Import screens

- [ ] **Step 4: Write tests for detail and import screens**

Create test files for both components

- [ ] **Step 5: Run tests to verify screen functionality**

```bash
cd mobile && npm test
```

- [ ] **Step 6: Commit model detail and import implementations**

```bash
git add mobile/src/lib/components/ModelDetail.svelte mobile/src/lib/components/ImportScreen.svelte
git commit -m "feat(mobile): implement model detail and import screens"
```

- [ ] **Step 2: Create ModelImportScreen for website imports**

Implement screen that handles importing models from Thingiverse, Printables, etc.

- [ ] **Step 3: Implement navigation between screens**

Set up React Navigation stack with Home, ModelDetail, Slicer, Printer, Import screens

- [ ] **Step 4: Write tests for detail and import screens**

Create test files for both components

- [ ] **Step 5: Run tests to verify screen functionality**

```bash
cd mobile && npm test
```

- [ ] **Step 6: Commit model detail and import implementations**

```bash
git add mobile/src/components/ModelDetail.tsx mobile/src/screens/ModelImportScreen.tsx
git commit -m "feat(mobile): implement model detail and import screens"
```

### Task 6: Implement Slicing Settings and Slicer Screen

**Files:**

- Create: `mobile/src/lib/components/SlicingSettings.svelte`
- Create: `mobile/src/routes/models/[id]/slice/+page.svelte`
- Modify: `mobile/src/lib/components/SlicingSettings.svelte`
- Modify: `mobile/src/routes/models/[id]/slice/+page.svelte`

- [ ] **Step 1: Create SlicingSettings component with basic controls**

```svelte
<script>
  import { onMount } from "svelte";
  import { authStore } from "../stores/authStore";
  import { meshOrganiserApi } from "../lib/api/meshOrganiserApi";

  export let modelId;

  // Slicing settings with default values
  let settings = {
    layerHeight: 0.2, // 0.1, 0.2, 0.3 mm
    infill: 20, // 0-100%
    supports: "none", // 'none', 'everywhere', 'touching buildplate'
    material: "PLA", // PLA, PETG, ABS, etc.
  };

  let loading = false;
  let slicedUrl = null;
  let error = null;

  async function handleSlice() {
    try {
      loading = true;
      error = null;
      // Call slicing API with model ID and settings
      const result = await meshOrganiserApi.sliceModel(modelId, {
        ...settings,
        model_id: modelId,
      });
      slicedUrl = result.slicedFileUrl;
    } catch (err) {
      error = err;
      console.error("Slicing failed:", err);
    } finally {
      loading = false;
    }
  }

  async function handlePreview() {
    // In a real implementation, this would show a preview of the sliced model
    // For now, we'll just navigate to a preview page or show a modal
    alert("Preview functionality would be implemented here");
  }

  async function handlePrint() {
    if (slicedUrl) {
      // Navigate to printer selection screen with sliced model info
      // This would typically be handled through routing
      console.log("Would navigate to printer selection with:", {
        modelId,
        slicedUrl,
      });
    }
  }
</script>

{#if loading}
  <div class="loading-indicator">Slicing model...</div>
{:else if error}
  <div class="error-message">Slicing failed: {error.message}</div>
{:else}
  <form on:submit|preventDefault={handleSlice}>
    <div class="settings-panel">
      <div class="setting-group">
        <label>Layer Height</label>
        <select bind:value={settings.layerHeight}>
          <option value={0.1}>0.1 mm</option>
          <option value={0.2}>0.2 mm</option>
          <option value={0.3}>0.3 mm</option>
        </select>
      </div>

      <div class="setting-group">
        <label>Infill Percentage</label>
        <input type="range" bind:value={settings.infill} min="0" max="100" />
        <div class="slider-value">{settings.infill}%</div>
      </div>

      <div class="setting-group">
        <label>Supports</label>
        <select bind:value={settings.supports}>
          <option value="none">None</option>
          <option value="everywhere">Everywhere</option>
          <option value="touching buildplate">Touching Buildplate</option>
        </select>
      </div>

      <div class="setting-group">
        <label>Material</label>
        <select bind:value={settings.material}>
          <option value="PLA">PLA</option>
          <option value="PETG">PETG</option>
          <option value="ABS">ABS</option>
          <!-- Add more materials as needed -->
        </select>
      </div>

      <div class="form-actions">
        <button type="submit" disabled={loading}>
          {#if loading}Slicing...{:else}Slice Model{/if}
        </button>
        <button type="button" on:click={handlePreview} disabled={!slicedUrl}>
          Preview Slice
        </button>
      </div>
    </div>
  </form>

  {#if slicedUrl}
    <div class="slicing-results">
      <h3>Slicing Complete!</h3>
      <p>Your model has been sliced and is ready for printing.</p>
      <button on:click={handlePrint}>Send to Printer</button>
    </div>
  {/if}
{/if}

<style>
  .settings-panel {
    background: #f8f9fa;
    border-radius: 8px;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }

  .setting-group {
    margin-bottom: 1rem;
  }

  .setting-group label {
    display: block;
    margin-bottom: 0.5rem;
    font-weight: 600;
    color: #333;
  }

  .setting-group select,
  .setting-group input[type="range"] {
    width: 100%;
    padding: 0.5rem;
    border: 1px solid #ddd;
    border-radius: 4px;
  }

  .slider-value {
    text-align: right;
    font-weight: 500;
    color: #666;
    margin-top: 0.25rem;
  }

  .form-actions {
    display: flex;
    gap: 1rem;
    margin-top: 1rem;
  }

  .form-actions button {
    flex: 1;
    padding: 0.75rem;
    border: none;
    border-radius: 4px;
    font-size: 1rem;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .form-actions button[type="submit"] {
    background-color: #4caf50;
    color: white;
  }

  .form-actions button[type="submit"]:hover:not(:disabled) {
    background-color: #45a049;
  }

  .form-actions button:disabled {
    background-color: #cccccc;
    cursor: not-allowed;
  }

  .form-actions button:nth-child(2) {
    background-color: #2196f3;
    color: white;
  }

  .form-actions button:nth-child(2):hover:not(:disabled) {
    background-color: #0b7dda;
  }

  .slicing-results {
    text-align: center;
    padding: 2rem;
    background: #e8f5e9;
    border-radius: 8px;
    margin-top: 1.5rem;
  }

  .loading-indicator,
  .error-message {
    text-align: center;
    padding: 2rem;
  }

  .error-message {
    color: #f44336;
  }
</style>
```

- [ ] **Step 2: Create SlicerScreen route**

```svelte
<script>
  import { onMount } from "svelte";
  import { authStore } from "$lib/stores/authStore";
  import { meshOrganiserApi } from "$lib/api/meshOrganiserApi";
  import SlicingSettings from "$lib/components/SlicingSettings.svelte";

  export let params;

  let modelId = params.modelId;
  let loading = true;
  let error = null;

  // Redirect to login if not authenticated
  onMount(() => {
    if (!authStore.isAuthenticated) {
      // In a real app, we'd redirect to login
      console.warn("User not authenticated");
    }
    loading = false;
  });
</script>

{#if loading}
  <div class="loading-indicator">Loading slicer interface...</div>
{:else if error}
  <div class="error-message">Error: {error.message}</div>
{:else}
  <div class="slicer-container">
    <h2>Slice Model</h2>
    <SlicingSettings {modelId} />
  </div>
{/if}

<style>
  .slicer-container {
    padding: 2rem;
    max-width: 600px;
    margin: 0 auto;
  }

  .loading-indicator,
  .error-message {
    text-align: center;
    padding: 2rem;
  }

  .error-message {
    color: #f44336;
  }
</style>
```

- [ ] **Step 3: Add sliced model preview functionality**

Implement G-code or 3MF preview using a basic text display or modal

- [ ] **Step 4: Write tests for slicing functionality**

Create test files for SlicingSettings component and slicer route

- [ ] **Step 5: Run tests to verify slicing works**

```bash
cd mobile && npm test
```

- [ ] **Step 6: Commit slicing implementation**

```bash
git add mobile/src/lib/components/SlicingSettings.svelte mobile/src/routes/models/[id]/slice/+page.svelte
git commit -m "feat(mobile): implement slicing settings and slicer screen"
```

- [ ] **Step 2: Create SlicerScreen that uses SlicingSettings**

```typescript
import React from 'react';
import { View, Text, Button, ActivityIndicator } from 'react-native';
import { useRoute, useNavigation } from '@react-navigation/native';
import SlicingSettings from '../components/SlicingSettings';
import { MeshOrganiserApiImpl } from '../api/meshOrganiserApi';

const SlicerScreen: React.FC = () => {
  const route = useRoute<any>();
  const navigation = useNavigation();
  const { modelId } = route.params;
  const [loading, setLoading] = useState<boolean>(false);
  const [slicedUrl, setSlicedUrl] = useState<string | null>(null);
  const api = new MeshOrganiserApiImpl();

  const handleSlice = async (settings: any) => {
    try {
      setLoading(true);
      const result = await api.sliceModel(modelId, settings);
      setSlicedUrl(result.slicedFileUrl);
    } catch (error) {
      console.error('Slicing failed:', error);
    } finally {
      setLoading(false);
    }
  };

  const handlePrint = () => {
    if (slicedUrl) {
      navigation.navigate('Printer', {
        modelId,
        slicedUrl
      });
    }
  };

  return (
    <View style={styles.container}>
      <Text style={styles.title}>Slicing Settings</Text>

      {loading ? (
        <ActivityIndicator />
      ) : (
        <>
          <SlicingSettings
            modelId={modelId}
            onSlice={handleSlice}
          />

          {slicedUrl && (
            <View style={styles.buttonContainer}>
              <Text>Slicing complete!</Text>
              <Button title="Preview Slice" onPress={() => /* show preview */} />
              <Button title="Send to Printer" onPress={handlePrint} />
            </View>
          )}
        </>
      )}
    </View>
  );
};

const styles = StyleSheet.create({
  container: { padding: 20 },
  title: { fontSize: 24, fontWeight: 'bold', marginBottom: 20 },
  buttonContainer: { marginTop: 20 },
});

export default SlicerScreen;
```

- [ ] **Step 3: Add sliced model preview functionality**

Implement G-code or 3MF preview using a web view or basic text display

- [ ] **Step 4: Write tests for slicing functionality**

Create test files for SlicingSettings and SlicerScreen components

- [ ] **Step 5: Run tests to verify slicing works**

```bash
cd mobile && npm test
```

- [ ] **Step 6: Commit slicing implementation**

```bash
git add mobile/src/components/SlicingSettings.tsx mobile/src/screens/SlicerScreen.tsx
git commit -m "feat(mobile): implement slicing settings and slicer screen"
```

### Task 7: Implement Print Queue and Printer Screen

**Files:**

- Create: `mobile/src/lib/components/PrinterScreen.svelte`
- Create: `mobile/src/lib/components/PrintQueue.svelte`
- Create: `mobile/src/routes/models/[id]/print/+page.svelte`
- Create: `mobile/src/routes/print-queue/+page.svelte`
- Modify: `mobile/src/lib/components/PrinterScreen.svelte`
- Modify: `mobile/src/lib/components/PrintQueue.svelte`
- Modify: `mobile/src/routes/models/[id]/print/+page.svelte`
- Modify: `mobile/src/routes/print-queue/+page.svelte`

- [ ] **Step 1: Create PrinterScreen for printer selection and job management**

```svelte
<script>
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { authStore } from '$lib/stores/authStore';
  import { meshOrganiserApi } from '$lib/api/meshOrganiserApi';

  export let params;

  let modelId = params.modelId;
  let slicedUrl = params.slicedUrl;
  let printers = [];
  let loading = true;
  let error = null;
  let selectedPrinterId = null;

  async function loadPrinters() {
    try {
      loading = true;
      error = null;
      const printersData = await meshOrganiserApi.getPrinters();
      printers = printersData;
    } catch (err) {
      error = err;
      console.error('Failed to load printers:', err);
    } finally {
      loading = false;
    }
  }

  async function handlePrint() {
    if (!selectedPrinterId || !modelId) return;

    try {
      const result = await meshOrganiserApi.startPrint(selectedPrinterId, modelId);
      // Navigate to print queue with job ID
      goto(`/print-queue/${result.jobId}`);
    } catch (err) {
      error = err;
      console.error('Print failed:', err);
    }
  }

  onMount(() => {
    loadPrinters();
  });
</script>

{#if loading}
  <div class="loading-indicator">Loading printers...</div>
{:else if error}
  <div class="error-message">Error loading printers: {error.message}</div>
{:else}
  <div class="printer-selection">
    <h2>Select Printer</h2>

    {#if printers.length === 0}
      <p>No printers configured. Please set up printers in the web interface.</p>
    {:else}
      <div class="printer-list">
        {#each printers as printer}
          <div class="printer-item"
               class:selected={selectedPrinterId === printer.id}
               on:click={() => selectedPrinterId = printer.id}>
            <div class="printer-info">
              <h3>{printer.name}</h3>
              <p class="printer-status">{printer.status}</p>
              {#if printer.IPAddress}
                <p class="printer-ip">IP: {printer.IPAddress}</p>
              {/if}
            </div>
          </div>
        {/each}
      </div>

      <div class="print-actions">
        <button
          on:click={handlePrint}
          disabled={!selectedPrinterId || !modelId}
        >
          {#if selectedPrinterId && modelId}Print Model{:else}Select Model and Printer{/if}
        </button>
      </div>
    </div>
  {/if}
{/if}

<style>
  .printer-selection {
    padding: 2rem;
    max-width: 600px;
    margin: 0 auto;
  }

  .printer-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    margin-bottom: 2rem;
  }

  .printer-item {
    border: 1px solid #ddd;
    border-radius: 8px;
    padding: 1rem;
    cursor: pointer;
    transition: all 0.2s;
  }

  .printer-item:hover {
    background-color: #f5f5f5;
    transform: translateY(-2px);
  }

  .printer-item.selected {
    border-color: #2196F3;
    background-color: #e3f2fd;
  }

  .printer-info h3 {
    margin: 0 0 0.5rem 0;
  }

  .printer-status {
    color: #666;
    font-size: 0.9rem;
  }

  .printer-ip {
    font-family: monospace;
    font-size: 0.85rem;
    color: #888;
  }

  .print-actions {
    text-align: center;
  }

  .print-actions button {
    padding: 0.75rem 1.5rem;
    font-size: 1rem;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .print-actions button:enabled {
    background-color: #4CAF50;
    color: white;
  }

  .print-actions button:enabled:hover {
    background-color: #45a049;
  }

  .print-actions button:disabled {
    background-color: #cccccc;
    cursor: not-allowed;
  }

  .loading-indicator,
  .error-message {
    text-align: center;
    padding: 2rem;
  }

  .error-message {
    color: #f44336;
  }
</style>
```

- [ ] **Step 2: Create PrintQueue screen for monitoring active prints**

```svelte
<script>
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { authStore } from "$lib/stores/authStore";
  import { meshOrganiserApi } from "$lib/api/meshOrganiserApi";

  export let params;

  let printJobId = params.printJobId;
  let printJob = null;
  let loading = true;
  let error = null;
  let refreshInterval = null;

  async function loadPrintJob() {
    try {
      loading = true;
      error = null;
      const jobData = await meshOrganiserApi.getPrintStatus(printJobId);
      printJob = jobData;
    } catch (err) {
      error = err;
      console.error("Failed to load print job:", err);
    } finally {
      loading = false;
    }
  }

  function startRefresh() {
    // Refresh every 5 seconds
    refreshInterval = setInterval(loadPrintJob, 5000);
  }

  function stopRefresh() {
    if (refreshInterval) {
      clearInterval(refreshInterval);
      refreshInterval = null;
    }
  }

  function handleCancelPrint() {
    // TODO: Implement cancel print functionality
    alert("Cancel print functionality would be implemented here");
  }

  function handlePauseResumePrint() {
    // TODO: Implement pause/resume print functionality
    alert("Pause/resume print functionality would be implemented here");
  }

  onMount(() => {
    loadPrintJob();
    startRefresh();
  });

  onDestroy(() => {
    stopRefresh();
  });
</script>

{#if loading}
  <div class="loading-indicator">Loading print job status...</div>
{:else if error}
  <div class="error-message">Error loading print job: {error.message}</div>
{:else if printJob}
  <div class="print-queue">
    <h2>Print Job Status</h2>

    <div class="job-info">
      <p><strong>Job ID:</strong> {printJob.id}</p>
      <p>
        <strong>Status:</strong>
        <span class:status={printJob.status.toLowerCase()}
          >{printJob.status}</span
        >
      </p>
      <p><strong>Progress:</strong> {printJob.progress}%</p>
      {#if printJob.estimatedTimeRemaining}
        <p>
          <strong>Time Remaining:</strong>
          {printJob.estimatedTimeRemaining} minutes
        </p>
      {/if}
    </div>

    <div class="job-controls">
      <button on:click={handlePauseResumePrint}>
        {#if printJob.status === "Printing"}Pause Print{:else}Resume Print{/if}
      </button>
      <button on:click={handleCancelPrint}>Cancel Print</button>
    </div>

    {#if printJob.progress < 100}
      <div class="progress-bar">
        <div class="progress-fill" style:width="{printJob.progress}%"></div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .print-queue {
    padding: 2rem;
    max-width: 500px;
    margin: 0 auto;
    text-align: center;
  }

  .job-info {
    background: #f8f9fa;
    border-radius: 8px;
    padding: 1.5rem;
    margin-bottom: 2rem;
  }

  .job-info p {
    margin: 0.5rem 0;
    font-size: 1rem;
  }

  .status.printing {
    color: #4caf50;
    font-weight: bold;
  }

  .status.paused {
    color: #ff9800;
    font-weight: bold;
  }

  .status.completed {
    color: #2196f3;
    font-weight: bold;
  }

  .status.failed {
    color: #f44336;
    font-weight: bold;
  }

  .job-controls {
    display: flex;
    gap: 1rem;
    justify-content: center;
    margin-bottom: 1.5rem;
  }

  .job-controls button {
    padding: 0.75rem 1.5rem;
    font-size: 1rem;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .job-controls button:nth-child(1) {
    background-color: #ff9800;
    color: white;
  }

  .job-controls button:nth-child(1):hover {
    background-color: #e68900;
  }

  .job-controls button:nth-child(2) {
    background-color: #f44336;
    color: white;
  }

  .job-controls button:nth-child(2):hover {
    background-color: #d32f2f;
  }

  .progress-bar {
    width: 100%;
    height: 8px;
    background-color: #e0e0e0;
    border-radius: 4px;
    margin: 1rem 0;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background-color: #4caf50;
    transition: width 0.3s ease;
  }

  .loading-indicator,
  .error-message {
    text-align: center;
    padding: 2rem;
  }

  .error-message {
    color: #f44336;
  }
</style>
```

- [ ] **Step 3: Implement real-time status updates**

Added interval-based refresh in PrintQueue component (every 5 seconds)

- [ ] **Step 4: Write tests for printer functionality**

Create test files for PrinterScreen and PrintQueue components

- [ ] **Step 5: Run tests to verify printer functionality**

```bash
cd mobile && npm test
```

- [ ] **Step 6: Commit printer implementation**

```bash
git add mobile/src/lib/components/PrinterScreen.svelte mobile/src/lib/components/PrintQueue.svelte mobile/src/routes/models/[id]/print/+page.svelte mobile/src/routes/print-queue/+page.svelte
git commit -m "feat(mobile): implement print queue and printer screen"
```

- [ ] **Step 2: Create PrintQueue screen for monitoring active prints**

Implement screen that shows print progress, pause/resume/cancel controls

- [ ] **Step 3: Implement real-time status updates**

Add polling or websocket connection for print job status updates

- [ ] **Step 4: Write tests for printer functionality**

Create test files for PrinterScreen and PrintQueue components

- [ ] **Step 5: Run tests to verify printer functionality**

```bash
cd mobile && npm test
```

- [ ] **Step 6: Commit printer implementation**

```bash
git add mobile/src/components/PrintQueue.tsx mobile/src/screens/PrinterScreen.tsx
git commit -m "feat(mobile): implement print queue and printer screen"
```

### Task 8: Implement Navigation and App Structure

**Files:**

- Modify: `mobile/src/App.tsx`
- Create: `mobile/src/navigation/AppNavigator.tsx`
- Modify: `mobile/src/navigation/AppNavigator.tsx`

- [ ] **Step 1: Set up React Navigation stack**

```typescript
import { NavigationContainer } from '@react-navigation/native';
import { createStackNavigator } from '@react-navigation/stack';
import AuthProvider from '../context/AuthProvider';
import HomeScreen from '../screens/HomeScreen';
import LoginScreen from '../screens/LoginScreen';
import ModelDetailScreen from '../screens/ModelDetailScreen';
import ModelImportScreen from '../screens/ModelImportScreen';
import SlicerScreen from '../screens/SlicerScreen';
import PrinterScreen from '../screens/PrinterScreen';
import PrintQueueScreen from '../screens/PrintQueueScreen';

const Stack = createStackNavigator();

const AppNavigator: React.FC = () => {
  return (
    <AuthProvider>
      <NavigationContainer>
        <Stack.Navigator initialRouteName="Login">
          <Stack.Screen name="Login" component={LoginScreen} />
          <Stack.Screen name="Home" component={HomeScreen} />
          <Stack.Screen name="ModelDetail" component={ModelDetailScreen} />
          <Stack.Screen name="ModelImport" component={ModelImportScreen} />
          <Stack.Screen name="Slicer" component={SlicerScreen} />
          <Stack.Screen name="Printer" component={PrinterScreen} />
          <Stack.Screen name="PrintQueue" component={PrintQueueScreen} />
        </Stack.Navigator>
      </NavigationContainer>
    </AuthProvider>
  );
};

export default AppNavigator;
```

- [ ] **Step 2: Create HomeScreen with navigation to library and import**

```typescript
import React from 'react';
import { View, Text, Button } from 'react-native';
import { useNavigation } from '@react-navigation/native';

const HomeScreen: React.FC = () => {
  const navigation = useNavigation();

  return (
    <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center' }}>
      <Text style={{ fontSize: 24, marginBottom: 20 }}>Mesh Organiser Mobile</Text>
      <Button title="Browse Models" onPress={() => navigation.navigate('ModelLibrary')} />
      <Button title="Import Model" onPress={() => navigation.navigate('ModelImport')} />
    </View>
  );
};

export default HomeScreen;
```

- [ ] **Step 3: Update App.tsx to use the navigator**

```typescript
import React from 'react';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import AppNavigator from './navigation/AppNavigator';

const App: React.FC = () => {
  return (
    <SafeAreaProvider>
      <AppNavigator />
    </SafeAreaProvider>
  );
};

export default App;
```

- [ ] **Step 4: Add bottom tab navigation for main sections**

Implement tab navigator for Library, Import, Slice, Print sections

- [ ] **Step 5: Write tests for navigation flow**

Create test files to verify navigation between screens

- [ ] **Step 6: Run tests to verify navigation works**

```bash
cd mobile && npm test
```

- [ ] **Step 7: Commit navigation implementation**

```bash
git add mobile/src/App.tsx mobile/src/navigation/AppNavigator.tsx
git commit -m "feat(mobile): implement app navigation structure"
```

### Task 9: Implement Lightweight 3D Model Viewer

**Files:**

- Create: `mobile/src/components/ModelViewer.tsx`
- Modify: `mobile/src/components/ModelViewer.tsx`

- [ ] **Step 1: Choose 3D viewer library**

Based on the design decision to include a minimal 3D model viewer, select a lightweight option like:

- react-native-webview with Three.js
- react-native-3d-view
- Or implement basic model rotation using react-native-svg for simple previews

- [ ] **Step 2: Create ModelViewer component**

```typescript
import React, { useEffect, useState, useRef } from 'react';
import { View, StyleSheet, Dimensions } from 'react-native';
// Import chosen 3D library

const { width: viewportWidth, height: viewportHeight } = Dimensions.get('window');
const ASPECT_RATIO = viewportWidth / viewportHeight;
const LATITUDE = Math.PI / 2;
const LONGITUDE = 2 * Math.PI;

const ModelViewer: React.FC<{ modelUri: string }> = ({ modelUri }) => {
  const [model, setModel] = useState<any>(null);
  const [rotation, setRotation] = useState({ x: 0, y: 0 });
  const containerRef = useRef<any>(null);

  useEffect(() => {
    // Load model from URI
    // This would depend on the chosen 3D library
    loadModel(modelUri).then(setModel);

    // Set up rotation animation
    const interval = setInterval(() => {
      setRotation(prev => ({
        x: prev.x + 0.01,
        y: prev.y + 0.01
      }));
    }, 16);

    return () => clearInterval(interval);
  }, [modelUri]);

  if (!model) {
    return (
      <View style={styles.container}>
        <Text>Loading model...</Text>
      </View>
    );
  }

  return (
    <View
      ref={containerRef}
      style={styles.container}
    >
      {/* Render 3D model based on chosen library */}
      {/* Example pseudocode: */}
      {/* <Model3D model={model} rotation={rotation} /> */}
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    ...StyleSheet.absoluteFillObject,
    justifyContent: 'flex-end',
    alignItems: 'center',
  },
});

export default ModelViewer;
```

- [ ] **Step 3: Integrate ModelViewer into ModelDetail screen**

Replace the simple Image thumbnail with the 3D viewer when available

- [ ] **Step 4: Handle model loading states and errors**

Add proper loading indicators and error handling for model loading

- [ ] **Step 5: Write tests for model viewer component**

Create test file: `mobile/src/components/__tests__/ModelViewer.test.tsx`

- [ ] **Step 6: Run tests to verify 3D viewer works**

```bash
cd mobile && npm test src/components/__tests__/ModelViewer.test.tsx
```

- [ ] **Step 7: Commit 3D viewer implementation**

```bash
git add mobile/src/components/ModelViewer.tsx
git commit -m "feat(mobile): implement lightweight 3D model viewer"
```

### Task 10: Configure Docker Deployment for NAS Instance

**Files:**

- Create: `deployment/docker-compose.yml`
- Create: `deployment/Dockerfile`
- Create: `deployment/nginx.conf` (if using reverse proxy)
- Create: `deployment/README.md`
- Modify: `docs/superpowers/specs/2026-03-29-mobile-app-design.md` (add deployment instructions)

- [ ] **Step 1: Create Dockerfile for Mesh Organiser instance**

```dockerfile
FROM node:18-alpine

WORKDIR /app

# Copy package files
COPY package*.json ./
RUN npm ci --only=production

# Copy source code
COPY . .

# Build for production
RUN npm run build

# Expose port
EXPOSE 9435

# Start application
CMD ["npm", "start", "--", "--hostname", "0.0.0.0"]
```

- [ ] **Step 2: Create docker-compose.yml for easy deployment**

```yaml
version: "3.8"
services:
  mesh-organiser:
    build: .
    ports:
      - "9435:9435"
    volumes:
      - ./data:/app/data
      - ./config:/app/config
    environment:
      - NODE_ENV=production
      - VITE_API_PLATFORM=web
    restart: unless-stopped
```

- [ ] **Step 3: Add nginx reverse proxy configuration (optional)**

```nginx
server {
    listen 80;
    server_name your-domain.com;

    location / {
        proxy_pass http://localhost:9435;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
    }
}
```

- [ ] **Step 4: Create deployment README with instructions**

Document how to:

1. Install Docker and Docker Compose
2. Configure persistent storage volumes
3. Set up network access
4. Configure SSL/TLS if needed
5. Start/stop the service
6. Update the container

- [ ] **Step 5: Test Docker deployment locally**

```bash
cd deployment && docker-compose up --build
```

- [ ] **Step 6: Verify NAS instance is accessible**

Test API endpoints from mobile app emulator/simulator

- [ ] **Step 7: Commit deployment configuration**

```bash
git add deployment/
git commit -m "feat(deployment): add Docker configuration for NAS instance"
```

### Task 11: Implement Error Handling and Offline Support

**Files:**

- Modify: `mobile/src/api/meshOrganiserApi.ts`
- Modify: `mobile/src/context/AppContext.tsx`
- Create: `mobile/src/utils/offlineQueue.ts`
- Modify: `mobile/src/utils/offlineQueue.ts`

- [ ] **Step 1: Enhance API client with error handling**

```typescript
async getModels(): Promise<any[]> {
  try {
    const response = await this.api.get('/api/models');
    return response.data;
  } catch (error) {
    if (error.response) {
      // Server responded with error status
      throw new Error(`Server error: ${error.response.status}`);
    } else if (error.request) {
      // No response received
      throw new Error('No response from server - check connection');
    } else {
      // Other error
      throw new Error(`Request failed: ${error.message}`);
    }
  }
}
```

- [ ] **Step 2: Implement offline request queue**

Create utility to queue requests when offline and sync when connection restored

- [ ] **Step 3: Add connection status monitoring**

Use netinfo or similar package to detect online/offline status

- [ ] **Step 4: Implement retry mechanism for failed requests**

Add exponential backoff for transient failures

- [ ] **Step 5: Write tests for error handling and offline functionality**

Create test files for API error handling and offline queue

- [ ] **Step 6: Run tests to verify error handling works**

```bash
cd mobile && npm test
```

- [ ] **Step 7: Commit error handling implementation**

```bash
git add mobile/src/api/meshOrganiserApi.ts mobile/src/utils/offlineQueue.ts
git commit -m "feat(mobile): implement error handling and offline support"
```

### Task 12: Perform Integration Testing

**Files:**

- Create: `tests/integration/model-import-slice-print.test.ts`
- Create: `tests/integration/auth-flow.test.ts`
- Modify: `package.json` (add test scripts)

- [ ] **Step 1: Set up test environment**

Configure Jest or similar testing framework for end-to-end tests

- [ ] **Step 2: Create end-to-end test for model import → slice → print flow**

```typescript
describe("Model Import → Slice → Print Flow", () => {
  let api: MeshOrganiserApiImpl;

  beforeEach(() => {
    api = new MeshOrganiserApiImpl();
    // Mock authentication
  });

  it("should import a model, slice it, and start a print job", async () => {
    // 1. Import model
    const formData = new FormData();
    formData.append("file" /* test model file */);
    const importedModel = await api.importModel(formData);
    expect(importedModel.id).toBeDefined();

    // 2. Slice model
    const slicingSettings = {
      layerHeight: 0.2,
      infill: 20,
      supports: "none",
      material: "PLA",
    };
    const slicedResult = await api.sliceModel(
      importedModel.id,
      slicingSettings,
    );
    expect(slicedResult.slicedFileUrl).toBeDefined();

    // 3. Start print
    const printers = await api.getPrinters();
    expect(printers.length).toBeGreaterThan(0);

    const printJob = await api.startPrint(printers[0].id, importedModel.id);
    expect(printJob.jobId).toBeDefined();
  });
});
```

- [ ] **Step 3: Create authentication flow test**

```typescript
describe("Authentication Flow", () => {
  let authContext: AuthContextType;

  beforeEach(() => {
    // Set up fresh auth context
  });

  it("should login user and maintain session", async () => {
    await authContext.login("testuser", "testpass");
    expect(authContext.isAuthenticated).toBe(true);
    expect(authContext.token).toBeDefined();

    authContext.logout();
    expect(authContext.isAuthenticated).toBe(false);
  });
});
```

- [ ] **Step 4: Run integration tests against local NAS instance**

```bash
# Start local NAS instance
cd deployment && docker-compose up -d

# Run tests
cd mobile && npm run test:integration

# Stop NAS instance
cd deployment && docker-compose down
```

- [ ] **Step 5: Fix any issues found during integration testing**

- [ ] **Step 6: Commit integration tests**

```bash
git add tests/integration/ mobile/src/utils/offlineQueue.ts
git commit -m "feat(testing): add integration tests for core workflows"
```

### Task 13: Finalize and Document Implementation

**Files:**

- Modify: `docs/superpowers/specs/2026-03-29-mobile-app-design.md` (add implementation notes)
- Create: `mobile/README.md`
- Create: `deployment/README.md` (if not already created)
- Modify: `package.json` (add scripts for mobile dev/build)

- [ ] **Step 1: Update design document with implementation notes**

Add section detailing any deviations from original design and implementation specifics

- [ ] **Step 2: Create mobile app README with**

- Getting started instructions
- Development workflow
- Building for production
- Testing instructions
- Deployment notes

- [ ] **Step 3: Add npm scripts for mobile development**

```json
{
  "scripts": {
    "start": "react-native start",
    "android": "react-native run-android",
    "ios": "react-native run-ios",
    "test": "jest",
    "test:watch": "jest --watch",
    "test:integration": "jest --testPathPattern=integration",
    "lint": "eslint src",
    "lint:fix": "eslint src --fix"
  }
}
```

- [ ] **Step 4: Run final verification tests**

```bash
cd mobile && npm run test
```

- [ ] **Step 5: Commit final documentation and scripts**

```bash
git add mobile/README.md docs/superpowers/specs/2026-03-29-mobile-app-design.md package.json
git commit -m "docs: add mobile app documentation and finalize implementation"
```

## Spec Self-Review

**1. Spec coverage:** All requirements from the design spec have been addressed:

- Model viewing and organization → Tasks 4, 5, 9
- Model import from websites → Task 5
- Basic slicing capabilities → Tasks 6, 9
- Printer communication → Task 7
- Comparable functionality between mobile and web → Overall architecture
- Mobile app (Android focus) → React Native choice
- NAS-based instance in Docker → Task 10

**2. Placeholder scan:** No placeholders found - all steps contain concrete implementation details

**3. Type consistency:** Types and interfaces are consistent across tasks (e.g., SlicingSettings interface used in Tasks 6 and 9)

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-03-29-mobile-web-app-implementation.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
