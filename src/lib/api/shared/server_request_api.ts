export enum HttpMethod {
  GET = "GET",
  POST = "POST",
  PUT = "PUT",
  DELETE = "DELETE",
}

export const IServerRequestApi = Symbol("IServerRequestApi");

export interface IServerRequestApi {
  baseUrl: string;
  request<T>(
    endpoint: string,
    method: HttpMethod,
    data?: unknown,
    version?: string,
  ): Promise<T>;
  requestBinary(
    endpoint: string,
    method: HttpMethod,
    data?: unknown,
    version?: string,
  ): Promise<Uint8Array>;
  sendBinary<T>(
    endpoint: string,
    method: HttpMethod,
    data: File,
    version?: string,
    extra_data?: Record<string, string>,
  ): Promise<T>;
}
