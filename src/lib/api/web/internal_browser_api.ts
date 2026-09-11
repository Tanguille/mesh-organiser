import type { IInternalBrowserApi } from "../shared/internal_browser_api";

export class WebBrowserApi implements IInternalBrowserApi {
  async openInternalBrowser(url: string): Promise<void> {
    window.open(url);
  }
}
