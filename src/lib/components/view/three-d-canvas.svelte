<script lang="ts">
  import type { ClassValue } from "svelte/elements";

  import { Canvas } from "@threlte/core";
  import { BufferAttribute, BufferGeometry } from "three";

  import { getContainer } from "$lib/api/dependency_injection";
  import { FileType, IBlobApi } from "$lib/api/shared/blob_api";
  import type { Model } from "$lib/api/shared/model_api";
  import ThreeScene from "$lib/components/view/three-d-scene.svelte";
  import { configuration } from "$lib/configuration.svelte";
  import { loadModel } from "$lib/workers/parseModelWorker";
  import { untrack } from "svelte";
  import Spinner from "./spinner.svelte";

  const props: { model: Model; class?: ClassValue; autoRotate?: boolean } =
    $props();
  let geometry: BufferGeometry | null = $state.raw(null);
  let loading = $state(true);
  let error: string | null = $state(null);
  let loadGeneration = 0;
  /** Skip reload when parent re-renders the same model + blob (new object reference). */
  let lastSuccessfulGeometryKey = "";
  /** Avoid duplicate loads while a fetch for this key is already in flight. */
  let pendingGeometryKey = "";

  function geometryLoadKey(model: Model): string {
    return `${model.id}:${model.blob.sha256}`;
  }

  async function loadUsingWorker(
    buffer: Uint8Array,
    fileType: FileType,
  ): Promise<BufferGeometry | null> {
    const worker = new Worker(
      new URL("../../workers/parseModelWorker.js", import.meta.url),
      {
        type: "module",
      },
    );

    return new Promise((resolve, reject) => {
      worker.onmessage = (event) => {
        const { success, geometry, error } = event.data;
        let incomingGeometry = geometry;
        worker.terminate();

        if (success) {
          const geometry = new BufferGeometry();
          geometry.setAttribute(
            "position",
            new BufferAttribute(new Float32Array(incomingGeometry.position), 3),
          );

          if (incomingGeometry.normal) {
            geometry.setAttribute(
              "normal",
              new BufferAttribute(new Float32Array(incomingGeometry.normal), 3),
            );
          } else {
            geometry.computeVertexNormals();
          }

          geometry.computeBoundingSphere();
          resolve(geometry);
        } else {
          reject(new Error(error));
        }
      };

      let obj = { buffer, fileType };
      worker.postMessage(obj, [buffer.buffer]);
    });
  }

  async function load(model: Model, gen: number) {
    loading = true;
    error = null;
    let localGeometry: BufferGeometry | null = geometry;
    geometry = null;
    localGeometry?.dispose();

    try {
      let blobApi = getContainer().require<IBlobApi>(IBlobApi);
      let bytes = await blobApi.getBlobBytes(model.blob);

      if (gen !== loadGeneration) {
        return;
      }

      if (configuration.use_worker_for_model_parsing) {
        localGeometry = await loadUsingWorker(bytes, model.blob.filetype);
      } else {
        localGeometry = loadModel(bytes, model.blob.filetype);
      }

      if (gen !== loadGeneration) {
        localGeometry?.dispose();
        return;
      }

      geometry = localGeometry;
      lastSuccessfulGeometryKey = geometryLoadKey(model);
    } catch (err) {
      if (gen !== loadGeneration) {
        return;
      }
      console.warn("Failed to load model geometry:", err);
      error = err instanceof Error ? err.message : "Failed to load model";
    } finally {
      if (gen === loadGeneration) {
        loading = false;
        pendingGeometryKey = "";
      }
    }
  }

  $effect(() => {
    const snapshot = $state.snapshot(props.model);
    if (!snapshot) {
      return;
    }
    const key = geometryLoadKey(snapshot);
    if (key === lastSuccessfulGeometryKey && geometry) {
      return;
    }
    if (pendingGeometryKey === key) {
      return;
    }
    pendingGeometryKey = key;
    const gen = ++loadGeneration;
    untrack(() => load(snapshot, gen));
  });
</script>

<div class={props.class}>
  {#if geometry}
    <Canvas>
      <ThreeScene {geometry} autoRotate={props.autoRotate} />
    </Canvas>
  {:else if loading}
    <div class="m-auto flex h-full flex-col items-center justify-center gap-3">
      <span class="text-xl">Loading model...</span>
      <Spinner />
    </div>
  {:else if error}
    <div class="m-auto flex h-full flex-col items-center justify-center gap-3">
      <span class="text-xl text-destructive">Failed to load model</span>
      <span class="text-sm text-muted-foreground">{error}</span>
    </div>
  {:else}
    <div class="m-auto flex h-full flex-col items-center justify-center gap-3">
      <span class="text-xl text-muted-foreground">No 3D preview available</span>
    </div>
  {/if}
</div>
