export interface SlicingSettings {
  layerHeight: number; // 0.1, 0.2, 0.3
  infill: number; // 0-100
  supports: 'none' | 'everywhere' | 'touching buildplate';
  material: string; // PLA, PETG, ABS, etc.
}

export interface SliceResult {
  success: boolean;
  slicedFileUrl: string;
  printTimeEstimate: number; // in minutes
  filamentUsed: number; // in grams
}

export interface SlicerEntry {
  slicer: string;
  installed: boolean;
}