import axios from 'axios';
import type { SlicerEntry } from '$lib/shared/slicer_api';

// Import shared interfaces directly from their source files to avoid circular dependency issues
import type { Blob } from '$lib/shared/blob_api';
import type { GroupMeta } from '$lib/shared/group_api';
import type { LabelMeta } from '$lib/shared/label_api';
import type { ModelFlags, Model } from '$lib/shared/model_api';
import { ModelOrderBy } from '$lib/shared/model_api';
import type { SlicingSettings, SliceResult } from '$lib/shared/slicer_api';
import { FileType } from '$lib/shared/blob_api';

// Define printer-related interfaces locally since they're not in shared API
export interface Printer {
  id: number;
  name: string;
  status: string; // idle, printing, paused, error, etc.
}

export interface PrintJob {
  id: string;
  status: string; // pending, printing, paused, completed, failed
  progress: number; // 0-100
}

export interface MeshOrganiserApi {
  getModels(
    modelIds: number[] | null,
    groupIds: number[] | null,
    labelIds: number[] | null,
    orderBy: ModelOrderBy,
    textSearch: string | null,
    page: number,
    pageSize: number,
    flags: ModelFlags | null
  ): Promise<Model[]>;
   
  getModel(id: number): Promise<Model>;
   
  importModel(formData: FormData): Promise<Model>;
   
  sliceModel(modelId: number, settings: SlicingSettings): Promise<SliceResult>;
   
  getPrinters(): Promise<Printer[]>;
   
  startPrint(printerId: number, modelId: number): Promise<PrintJob>;
   
  getPrintStatus(printJobId: string): Promise<PrintJob>;
  
  getPrintJobs(): Promise<PrintJob[]>;
}

// Implementation of the API client
class MeshOrganiserApiImpl implements MeshOrganiserApi {
  private api: ReturnType<typeof axios.create>;
  
  constructor() {
    // Determine base URL based on environment
    const API_BASE_URL = import.meta.env.DEV
      ? 'http://localhost:9435'
      : 'http://your-nas-ip:9435';
    
    this.api = axios.create({
      baseURL: API_BASE_URL,
      headers: {
        'Content-Type': 'application/json',
      },
    });
    
    // Add request interceptor for authentication
    this.api.interceptors.request.use((config) => {
      const token = localStorage.getItem('auth_token');
      if (token) {
        config.headers.Authorization = `Bearer ${token}`;
      }
      return config;
    });
    
    // Add response interceptor for error handling
    this.api.interceptors.response.use(
      (response) => response,
      (error) => {
        // Handle common error cases
        if (error.response) {
          // Server responded with error status
          console.error(`API Error: ${error.response.status} - ${error.response.data}`);
        } else if (error.request) {
          // No response received
          console.error('API Error: No response from server');
        } else {
          // Other error
          console.error(`API Error: ${error.message}`);
        }
        return Promise.reject(error);
      }
    );
  }
  
  async getModels(
    modelIds: number[] | null = null,
    groupIds: number[] | null = null,
    labelIds: number[] | null = null,
    orderBy: ModelOrderBy = ModelOrderBy.AddedDesc,
    textSearch: string | null = null,
    page: number = 1,
    pageSize: number = 50,
    flags: ModelFlags | null = null
  ): Promise<Model[]> {
    const response = await this.api.get('/api/models', {
      params: {
        model_ids: modelIds,
        group_ids: groupIds,
        label_ids: labelIds,
        order_by: orderBy,
        text_search: textSearch,
        page,
        page_size: pageSize,
        flags: flags ? Object.entries(flags)
          .filter(([, value]) => value)
          .map(([key]) => key) : undefined
      }
    });
    // Ensure we return an array even if API returns something else
    return Array.isArray(response.data) ? response.data : [];
  }
  
    async getModel(id: number): Promise<Model> {
      const response = await this.api.get(`/api/models/${id}`);
    // Ensure we return a proper Model object
    if (response.data) {
      return response.data as Model;
    }
    // Return a properly typed fallback object
    return {
      id: 0,
      name: '',
      blob: { id: 0, sha256: '', filetype: FileType.STL, size: 0, added: new Date(), name: '', mimeType: '' },
      link: null,
      description: null,
      added: new Date(),
      lastModified: new Date(),
      group: null,
      labels: [],
      flags: { printed: false, favorite: false },
      uniqueGlobalId: ''
    } as Model;
    }
  
  async importModel(formData: FormData): Promise<Model> {
    const response = await this.api.post('/api/models', formData, {
      headers: {
        'Content-Type': 'multipart/form-data',
      },
    });
    // Ensure we return a proper Model object
    if (response.data) {
      return response.data as Model;
    }
    // Return a properly typed fallback object
    return {
      id: 0,
      name: '',
      blob: { id: 0, sha256: '', filetype: FileType.STL, size: 0, added: new Date(), name: '', mimeType: '' },
      link: null,
      description: null,
      added: new Date(),
      lastModified: new Date(),
      group: null,
      labels: [],
      flags: { printed: false, favorite: false },
      uniqueGlobalId: ''
    } as Model;
  }
  
  async sliceModel(modelId: number, settings: SlicingSettings): Promise<SliceResult> {
    const response = await this.api.post(`/api/slicer/slice`, {
      model_id: modelId,
      ...settings,
    });
    // Ensure we return a proper SliceResult object
    if (response.data) {
      return response.data as SliceResult;
    }
    // Return a properly typed fallback object
    return {
      success: false,
      slicedFileUrl: '',
      printTimeEstimate: 0,
      filamentUsed: 0
    } as SliceResult;
  }
  
  async getPrinters(): Promise<Printer[]> {
    const response = await this.api.get('/api/printers');
    // Ensure we return an array even if API returns something else
    return Array.isArray(response.data) ? response.data : [];
  }
  
  async startPrint(printerId: number, modelId: number): Promise<PrintJob> {
    const response = await this.api.post(`/api/printers/${printerId}/print`, {
      model_id: modelId,
    });
    // Ensure we return a proper PrintJob object
    if (response.data) {
      return response.data as PrintJob;
    }
    // Return a properly typed fallback object
    return {
      id: '',
      status: 'pending',
      progress: 0
    } as PrintJob;
  }
  
    async getPrintStatus(printJobId: string): Promise<PrintJob> {
      const response = await this.api.get(`/api/printers/status/${printJobId}`);
      // Ensure we return a proper PrintJob object
      if (response.data) {
        return response.data as PrintJob;
      }
      // Return a properly typed fallback object
      return {
        id: printJobId,
        status: 'unknown',
        progress: 0
      } as PrintJob;
    }

    async getPrintJobs(): Promise<PrintJob[]> {
      const response = await this.api.get('/api/printers/jobs');
      // Ensure we return an array even if API returns something else
      return Array.isArray(response.data) ? response.data : [];
    }
}

// Export a singleton instance
export const meshOrganiserApi = new MeshOrganiserApiImpl();
export default meshOrganiserApi;