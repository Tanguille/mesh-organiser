import { displayAppVersion } from "../shared/display_app_version";
import { IHostApi, Platform } from "../shared/host_api";

// Shared by the web app and the demo, which differ only in the platform.
export class WebHostApi implements IHostApi {
  private platform: Platform;

  constructor(platform = Platform.WebApp) {
    this.platform = platform;
  }

  async getPlatform(): Promise<Platform> {
    return this.platform;
  }

  async getVersion(): Promise<string> {
    return displayAppVersion(import.meta.env.VITE_APP_VERSION);
  }
}
