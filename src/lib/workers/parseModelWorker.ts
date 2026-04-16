import { BufferGeometry, Mesh, Group, type Object3D } from "three";
import {
  mergeGeometries,
  toCreasedNormals,
} from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
// @ts-expect-error - threejs-webworker-3mf-loader has no type definitions
import { ThreeMFLoader } from "threejs-webworker-3mf-loader";
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import { FileType } from "$lib/api/shared/blob_api";
import { isValidWorkerMessage } from "./parseModelWorkerMessage.js";

/** Single decoder avoids per-parse allocation of decoder state (minor; hot path for OBJ). */
const utf8Decoder = new TextDecoder("utf-8");

/** Binary loaders expect an `ArrayBuffer` of exactly this view (subarrays share a parent buffer). */
function uint8ViewToArrayBuffer(buffer: Uint8Array): ArrayBuffer {
  return buffer.buffer.slice(
    buffer.byteOffset,
    buffer.byteOffset + buffer.byteLength,
  ) as ArrayBuffer;
}

function convertGeometry(group: Group): BufferGeometry | null {
  const geometries: BufferGeometry[] = [];
  group.updateMatrixWorld(true);

  group.traverse((child: Object3D) => {
    if (child.type === "Mesh") {
      const mesh = child as Mesh;
      const clone = mesh.geometry.clone();
      clone.applyMatrix4(mesh.matrixWorld);
      geometries.push(clone.index ? clone.toNonIndexed() : clone);
    }
  });

  if (geometries.length === 0) {
    return null;
  }

  let merged: BufferGeometry;
  try {
    merged = mergeGeometries(geometries, false);
  } catch (error) {
    console.error("Error merging geometries:", error);
    return null;
  }

  geometries.forEach((geometry) => {
    geometry.dispose();
  });

  return merged;
}

export function loadModel(
  buffer: Uint8Array,
  fileType: FileType,
): BufferGeometry | null {
  let parsedGeometry: BufferGeometry | null = null;

  try {
    if (fileType === FileType.STL) {
      if (import.meta.env.DEV) {
        console.debug("[parseModelWorker] STL buffer length:", buffer.length);
      }
      const loader = new STLLoader();
      parsedGeometry = loader.parse(uint8ViewToArrayBuffer(buffer));
      if (import.meta.env.DEV) {
        console.debug("[parseModelWorker] STL parse done");
      }
    } else if (fileType === FileType.THREEMF) {
      if (import.meta.env.DEV) {
        console.debug(
          "[parseModelWorker] ThreeMF buffer length:",
          buffer.length,
        );
      }
      const loader = new ThreeMFLoader();
      const result = loader.parse(uint8ViewToArrayBuffer(buffer));

      parsedGeometry = convertGeometry(result) || new BufferGeometry();
    } else if (fileType === FileType.OBJ) {
      // OBJ is a text format. Three.js OBJLoader only accepts a string, so we must UTF-8 decode
      // the whole buffer (O(n) time and memory). Large files are usually dominated by decode +
      // parse, not postMessage. Use dev timings below to confirm in your environment.
      const tDecodeStart = performance.now();
      const text = utf8Decoder.decode(buffer);
      const tParseStart = performance.now();
      const loader = new OBJLoader();
      const result = loader.parse(text);
      const tConvertStart = performance.now();
      parsedGeometry = convertGeometry(result) || new BufferGeometry();
      const tEnd = performance.now();
      if (import.meta.env.DEV) {
        console.debug("[parseModelWorker] OBJ ms", {
          decode: tParseStart - tDecodeStart,
          parse: tConvertStart - tParseStart,
          convertGeometry: tEnd - tConvertStart,
          total: tEnd - tDecodeStart,
          bufferBytes: buffer.length,
          textChars: text.length,
        });
      }
    } else {
      console.error(
        "Unknown file type:",
        fileType,
        "available types:",
        Object.values(FileType),
      );
      parsedGeometry = null;
    }

    if (parsedGeometry) {
      parsedGeometry = toCreasedNormals(parsedGeometry, 0.1);
      parsedGeometry.computeBoundingSphere();
      parsedGeometry.center();
      parsedGeometry.rotateX(Math.PI / -2);
      if (import.meta.env.DEV) {
        console.debug("[parseModelWorker] geometry post-process done");
      }
    }

    return parsedGeometry ?? null;
  } catch (error) {
    console.error(
      "Error in loadModel:",
      error,
      "for fileType:",
      fileType,
      "buffer length:",
      buffer?.length ?? "(n/a)",
    );
    return null;
  }
}

self.onmessage = async (event: MessageEvent<unknown>) => {
  if (!isValidWorkerMessage(event.data)) {
    return;
  }
  const { buffer, fileType } = event.data;
  if (import.meta.env.DEV) {
    console.debug(
      "[parseModelWorker] message",
      fileType,
      "bytes",
      buffer.length,
    );
  }

  try {
    const geometry = loadModel(buffer, fileType);

    if (geometry) {
      // Check if geometry has valid position data
      if (geometry.attributes.position) {
        const position = geometry.attributes.position.array.buffer;
        const normal = geometry.attributes.normal?.array?.buffer || null;

        const transferables = [position];
        if (normal) {
          transferables.push(normal);
        }

        if (import.meta.env.DEV) {
          console.debug(
            "[parseModelWorker] vertices",
            geometry.attributes.position.count,
          );
        }
        self.postMessage(
          {
            success: true,
            geometry: {
              vertexCount: geometry.attributes.position.count,
              position: position,
              normal: normal,
            },
            error: null,
          },
          { transfer: transferables },
        );
      } else {
        // Geometry exists but has no position data (e.g., empty or unsupported)
        console.error(
          "Geometry parsed but has no position attributes for fileType:",
          fileType,
        );
        self.postMessage({
          success: false,
          geometry: null,
          error: `Model parsed but contains no geometry data (type: ${fileType})`,
        });
      }
    } else {
      console.error("loadModel returned null geometry for fileType:", fileType);
      const errorMsg = `Model format not supported or failed to parse (type: ${fileType}, buffer size: ${buffer.length})`;
      self.postMessage({
        success: false,
        geometry: null,
        error: errorMsg,
      });
    }
  } catch (error: unknown) {
    console.error("Worker error:", error, "for fileType:", fileType);
    const errorMsg = `Worker error: ${
      error instanceof Error ? error.message : String(error)
    } (type: ${fileType})`;
    self.postMessage({ success: false, geometry: null, error: errorMsg });
  }
};
