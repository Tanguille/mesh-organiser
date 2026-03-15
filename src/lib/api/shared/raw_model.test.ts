/**
 * Regression tests for shared raw model types and convertModelFlagsToRaw,
 * after moving this code from tauri-specific to shared (used by web, web_share, tauri).
 */

import { describe, it, expect } from "vitest";
import type { ModelFlags } from "./model_api";
import { convertModelFlagsToRaw } from "./raw_model";

describe("convertModelFlagsToRaw", () => {
	it("returns null when flags is null", () => {
		// Arrange
		const flags: ModelFlags | null = null;
		// Act
		const result = convertModelFlagsToRaw(flags);
		// Assert
		expect(result).toBeNull();
	});

	it("returns null when flags is empty (no printed or favorite)", () => {
		// Arrange
		const flags: ModelFlags = { printed: false, favorite: false };
		// Act
		const result = convertModelFlagsToRaw(flags);
		// Assert
		expect(result).toBeNull();
	});

	it("returns [\"Printed\"] when printed is true and favorite is false", () => {
		// Arrange
		const flags: ModelFlags = { printed: true, favorite: false };
		// Act
		const result = convertModelFlagsToRaw(flags);
		// Assert
		expect(result).toEqual(["Printed"]);
	});

	it("returns [\"Favorite\"] when printed is false and favorite is true", () => {
		// Arrange
		const flags: ModelFlags = { printed: false, favorite: true };
		// Act
		const result = convertModelFlagsToRaw(flags);
		// Assert
		expect(result).toEqual(["Favorite"]);
	});

	it("returns array containing both \"Printed\" and \"Favorite\" when both are true", () => {
		// Arrange
		const flags: ModelFlags = { printed: true, favorite: true };
		// Act
		const result = convertModelFlagsToRaw(flags);
		// Assert
		expect(result).toHaveLength(2);
		expect(result).toContain("Printed");
		expect(result).toContain("Favorite");
	});
});
