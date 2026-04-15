import { displayAppVersion } from "../shared/display_app_version";
import { IHostApi, Platform } from "../shared/host_api";

export class DemoHostApi implements IHostApi {
  async getPlatform(): Promise<Platform> {
    return Platform.DemoWebApp;
  }

  async getVersion(): Promise<string> {
    return displayAppVersion(import.meta.env.VITE_APP_VERSION);
  }
}
