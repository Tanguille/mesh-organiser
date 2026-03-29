import type { MeshOrganiserApi } from '$lib/api/meshOrganiserApi';

// Define argument types for each API method
type GetModelsArgs = [modelIds?: number[] | null, groupIds?: number[] | null, labelIds?: number[] | null, 
                     orderBy?: any, textSearch?: string | null, page?: number, pageSize?: number, flags?: any];
type GetModelArgs = [id: number];
type ImportModelArgs = [formData: FormData];
type SliceModelArgs = [modelId: number, settings: any];
type GetPrintersArgs = [];
type StartPrintArgs = [printerId: number, modelId: number];
type GetPrintStatusArgs = [printJobId: string];
type GetPrintJobsArgs = [];

// Interface for queued requests
interface QueuedRequest {
  id: string;
  timestamp: number;
  method: 'getModels' | 'getModel' | 'importModel' | 'sliceModel' | 'getPrinters' | 'startPrint' | 'getPrintStatus' | 'getPrintJobs';
  args: any[];
  resolve: (value: any) => void;
  reject: (reason?: any) => void;
}

class OfflineQueue {
  private queue: QueuedRequest[] = [];
  private isProcessing = false;
  private api: MeshOrganiserApi;
  
  constructor(api: MeshOrganiserApi) {
    this.api = api;
    
    // Listen for online/offline events
    if (typeof window !== 'undefined') {
      window.addEventListener('online', () => this.processQueue());
      window.addEventListener('offline', () => console.log('Device is offline'));
    }
    
    // Process queue immediately if online
    if (typeof navigator !== 'undefined' && navigator.onLine) {
      this.processQueue();
    }
  }
  
  private async processQueue() {
    if (this.isProcessing || typeof navigator === 'undefined' || !navigator.onLine || this.queue.length === 0) {
      return;
    }
    
    this.isProcessing = true;
    
    while (this.queue.length > 0 && typeof navigator !== 'undefined' && navigator.onLine) {
      const request = this.queue.shift();
      if (!request) continue;
      
      try {
        // Execute the queued request
        let result: any;
        switch (request.method) {
          case 'getModels':
            result = await this.api.getModels(...(request.args as GetModelsArgs));
            break;
          case 'getModel':
            result = await this.api.getModel(...(request.args as GetModelArgs));
            break;
          case 'importModel':
            result = await this.api.importModel(...(request.args as ImportModelArgs));
            break;
          case 'sliceModel':
            result = await this.api.sliceModel(...(request.args as SliceModelArgs));
            break;
          case 'getPrinters':
            result = await this.api.getPrinters(...(request.args as GetPrintersArgs));
            break;
          case 'startPrint':
            result = await this.api.startPrint(...(request.args as StartPrintArgs));
            break;
          case 'getPrintStatus':
            result = await this.api.getPrintStatus(...(request.args as GetPrintStatusArgs));
            break;
          case 'getPrintJobs':
            result = await this.api.getPrintJobs(...(request.args as GetPrintJobsArgs));
            break;
        }
        
        request.resolve(result);
      } catch (error) {
        request.reject(error);
      }
    }
    
    this.isProcessing = false;
    
    // If there are still items in the queue and we're online, continue processing
    if (this.queue.length > 0 && typeof navigator !== 'undefined' && navigator.onLine) {
      this.processQueue();
    }
  }
  
  // Wrapper methods for API calls that will queue when offline
  async getModels(...args: GetModelsArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'getModels',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        // Try to execute immediately if online
        this.api.getModels(...args).then(resolve).catch(reject);
      } else {
        // Queue the request
        this.queue.push(request);
        this.processQueue(); // Try to process if we just came online
      }
    });
  }
  
  async getModel(...args: GetModelArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'getModel',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.getModel(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  async importModel(...args: ImportModelArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'importModel',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.importModel(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  async sliceModel(...args: SliceModelArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'sliceModel',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.sliceModel(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  async getPrinters(...args: GetPrintersArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'getPrinters',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.getPrinters(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  async startPrint(...args: StartPrintArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'startPrint',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.startPrint(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  async getPrintStatus(...args: GetPrintStatusArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'getPrintStatus',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.getPrintStatus(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  async getPrintJobs(...args: GetPrintJobsArgs): Promise<any> {
    return new Promise((resolve, reject) => {
      const request: QueuedRequest = {
        id: Math.random().toString(36).substr(2, 9),
        timestamp: Date.now(),
        method: 'getPrintJobs',
        args,
        resolve,
        reject
      };
      
      if (typeof navigator !== 'undefined' && navigator.onLine) {
        this.api.getPrintJobs(...args).then(resolve).catch(reject);
      } else {
        this.queue.push(request);
        this.processQueue();
      }
    });
  }
  
  // Get queue length for debugging/monitoring
  getQueueLength(): number {
    return this.queue.length;
  }
  
  // Clear queue (useful for testing or reset)
  clearQueue(): void {
    this.queue = [];
  }
}

export default OfflineQueue;