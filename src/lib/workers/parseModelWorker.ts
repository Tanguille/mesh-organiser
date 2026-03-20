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

function convertGeometry(group: Group): BufferGeometry | null {
  const geometries: BufferGeometry[] = [];
  group.updateMatrixWorld(true);

  group.traverse((object: Object3D) => {
    if (object.type === "Mesh") {
      const mesh = object as Mesh;
      const clone = mesh.geometry.clone();
      clone.applyMatrix4(mesh.matrixWorld);
      geometries.push(clone.index ? clone.toNonIndexed() : clone);
    }
  });

  console.log(geometries);

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
  let localResult;

  try {
    if (fileType === FileType.STL) {
      console.log("Loading STL file, buffer length:", buffer.length);
      const loader = new STLLoader();
      localResult = loader.parse(buffer.buffer as ArrayBuffer);
      console.log("STL parsed successfully, result:", localResult);
    } else if (fileType === FileType.THREEMF) {
      console.log("Loading ThreeMF file, buffer length:", buffer.length);
      const loader = new ThreeMFLoader();
      const result = loader.parse(buffer.buffer as ArrayBuffer);
      console.log("ThreeMF loader result:", result);

      localResult = convertGeometry(result) || new BufferGeometry();
      console.log("ThreeMF convertGeometry result:", localResult);
    } else if (fileType === FileType.OBJ) {
      console.log("Loading OBJ file, buffer length:", buffer.length);
      const loader = new OBJLoader();
      // TODO: This is slow!
      const text = new TextDecoder("utf-8").decode(buffer);
      console.log(
        "OBJ text length:",
        text.length,
        "first 100 chars:",
        text.substring(0, 100),
      );
      const result = loader.parse(text);
      console.log("OBJ loader result:", result);

      localResult = convertGeometry(result) || new BufferGeometry();
      console.log("OBJ convertGeometry result:", localResult);
    } else {
      console.error(
        "Unknown file type:",
        fileType,
        "available types:",
        Object.values(FileType),
      );
      localResult = null;
    }

    if (localResult) {
      localResult = toCreasedNormals(localResult, 0.1);
      localResult.computeBoundingSphere();
      localResult.center();
      localResult.rotateX(Math.PI / -2);
      console.log("Model processed successfully");
    }

    return localResult || null;
  } catch (error) {
    console.error(
      "Error in loadModel:",
      error,
      "for fileType:",
      fileType,
      "buffer length:",
      buffer.length,
    );
    return null;
  }
}

self.onmessage = async (
  e: MessageEvent<{ buffer: Uint8Array; fileType: FileType }>,
) => {
  const { buffer, fileType } = e.data;
  console.log(
    "Worker received message, fileType:",
    fileType,
    "buffer length:",
    buffer.length,
  );

  try {
    const geometry = loadModel(buffer, fileType);
    console.log(
      "loadModel result:",
      geometry ? "geometry returned" : "null returned",
    );

    if (geometry) {
      // Check if geometry has valid position data
      if (geometry.attributes.position) {
        const position = geometry.attributes.position.array.buffer;
        const normal = geometry.attributes.normal?.array?.buffer || null;

        const transferables = [position];
        if (normal) {
          transferables.push(normal);
        }

        console.log(
          "Sending successful geometry data, vertices:",
          geometry.attributes.position.count,
        );
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
  } catch (error) {
    console.error("Worker error:", error, "for fileType:", fileType);
    const errorMsg = `Worker error: ${
      error instanceof Error ? error.message : String(error)
    } (type: ${fileType})`;
    self.postMessage({ success: false, geometry: null, error: errorMsg });
    throw error;
  }
};
