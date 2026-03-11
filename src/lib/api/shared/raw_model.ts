import type { ModelFlags } from "./model_api";

/** Raw blob shape (matches tauri blob response). */
export interface RawBlob {
	id: number;
	sha256: string;
	filetype: string;
	size: number;
	added: string;
}

/** Raw group meta shape (matches tauri group response). */
export interface RawGroupMeta {
	id: number;
	name: string;
	created: string;
	last_modified: string;
	resource_id: number | null;
	unique_global_id: string;
}

/** Raw label meta shape (matches tauri label response). */
export interface RawLabelMeta {
	id: number;
	name: string;
	color: number;
	unique_global_id: string;
	last_modified: string;
}

export interface RawModel {
	id: number;
	name: string;
	blob: RawBlob;
	link: string | null;
	description: string | null;
	added: string;
	last_modified: string;
	group: RawGroupMeta | null;
	labels: RawLabelMeta[];
	flags: string[];
	unique_global_id: string;
}

export function convertModelFlagsToRaw(flags: ModelFlags | null): string[] | null {
	if (flags === null) {
		return null;
	}

	const raw_flags: string[] = [];

	if (flags.printed) {
		raw_flags.push("Printed");
	}

	if (flags.favorite) {
		raw_flags.push("Favorite");
	}

	if (raw_flags.length === 0) {
		return null;
	}

	return raw_flags;
}
