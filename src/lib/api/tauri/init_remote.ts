import { fetch } from "@tauri-apps/plugin-http";
import { getContainer, resetContainer } from "../dependency_injection";
import { IServerRequestApi } from "../shared/server_request_api";
import {
  IAdminUserApi,
  IUserApi,
  IUserLoginApi,
  IUserLogoutApi,
  IUserManageSelfApi,
  IUserTokenApi,
} from "../shared/user_api";
import { WebUserLoginApi } from "../web/login";
import { ServerRequestApi } from "../web/request";
import { WebUserApi } from "../web/user";
import { WebBlobApi } from "../web/blob";
import { WebDiskUsageInfoApi } from "../web/disk_usage_info";
import { WebGroupApi } from "../web/group";
import { WebHostApi } from "../web/host";
import { WebLabelApi } from "../web/label";
import { WebModelApi } from "../web/model";
import { WebResourceApi } from "../web/resource";
import { WebSettingsApi } from "../web/settings";
import { WebImportApi } from "../web/web_import";
import {
  DefaultSidebarStateApi,
  ISidebarStateApi,
} from "../shared/sidebar_state_api";
import { DefaultDownloadApi, IDownloadApi } from "../shared/download_api";
import {
  configuration,
  currentUser as globalCurrentUser,
  panicState,
} from "$lib/configuration.svelte";
import { IBlobApi } from "../shared/blob_api";
import { IDiskUsageInfoApi } from "../shared/disk_usage_info_api";
import { IGroupApi } from "../shared/group_api";
import { IWebImportApi } from "../shared/web_import_api";
import { IHostApi } from "../shared/host_api";
import { ILabelApi } from "../shared/label_api";
import { IModelApi } from "../shared/model_api";
import { IResourceApi } from "../shared/resource_api";
import { ISettingsApi } from "../shared/settings_api";
import { WebBrowserApi } from "../web/internal_browser_api";
import { IInternalBrowserApi } from "../shared/internal_browser_api";
import { WebThreemfApi } from "../web/threemf";
import { IThreemfApi } from "../shared/threemf_api";
import { WebUserAdminApi } from "../web/user_admin";
import { WebShareApi } from "../web/share";
import { IShareApi } from "../shared/share_api";
import { ISlicerApi } from "../shared/slicer_api";
import { MobileRemoteSlicerApi } from "../web/remote_slicer";
import { getServerUrl } from "./server_url";

export async function initTauriRemoteApis(): Promise<void> {
  resetContainer();

  const persisted = await getServerUrl();
  const envUrl =
    (import.meta.env.VITE_MOBILE_SERVER_URL as string | undefined) ?? "";
  const raw = (persisted?.trim() || envUrl.trim() || "").trim();
  const baseUrl = raw.replace(/\/+$/, "");

  const container = getContainer();

  if (baseUrl.length === 0) {
    panicState.inPanic = true;
    panicState.message =
      "No remote server URL is configured.\nSet the server URL in the app settings (or VITE_MOBILE_SERVER_URL for development) and restart.";

    const request = new ServerRequestApi("", fetch, "include");
    const user = new WebUserApi(request);
    const login = new WebUserLoginApi(request);

    container.addSingleton(IServerRequestApi, request);
    container.addSingleton(IUserApi, user);
    container.addSingleton(IUserLoginApi, login);
    container.addSingleton(IUserLogoutApi, login);

    return;
  }

  panicState.inPanic = false;

  const request = new ServerRequestApi(baseUrl, fetch, "include");
  const user = new WebUserApi(request);
  const login = new WebUserLoginApi(request);

  container.addSingleton(IServerRequestApi, request);
  container.addSingleton(IUserApi, user);
  container.addSingleton(IUserLoginApi, login);
  container.addSingleton(IUserLogoutApi, login);

  if (!(await user.isAuthenticated())) {
    console.log("User is not authenticated");
    return;
  }

  let currentUser;

  try {
    currentUser = await user.getCurrentUser();
  } catch {
    console.log("User is not authenticated");
    return;
  }

  Object.assign(globalCurrentUser, currentUser);
  const blob = new WebBlobApi(request, currentUser);
  const diskUsageInfo = new WebDiskUsageInfoApi(request);
  const group = new WebGroupApi(request);
  const host = new WebHostApi();
  const label = new WebLabelApi(request);
  const model = new WebModelApi(request);
  const resource = new WebResourceApi(request);
  const settings = new WebSettingsApi();
  const importApi = new WebImportApi(request);
  const slicer = new MobileRemoteSlicerApi(blob, request);
  const sidebarApi = new DefaultSidebarStateApi();
  const downloadApi = new DefaultDownloadApi(blob);
  const internalBrowserApi = new WebBrowserApi();
  const threemf = new WebThreemfApi(request);
  const userAdmin = new WebUserAdminApi(request, currentUser);
  const shareApi = new WebShareApi(request);

  const config = await settings.getConfiguration();
  Object.assign(configuration, config);

  container.addSingleton(IBlobApi, blob);
  container.addSingleton(IDiskUsageInfoApi, diskUsageInfo);
  container.addSingleton(IGroupApi, group);
  container.addSingleton(ILabelApi, label);
  container.addSingleton(IModelApi, model);
  container.addSingleton(IResourceApi, resource);
  container.addSingleton(IWebImportApi, importApi);
  container.addSingleton(IHostApi, host);
  container.addSingleton(ISettingsApi, settings);
  container.addSingleton(ISlicerApi, slicer);
  container.addSingleton(ISidebarStateApi, sidebarApi);
  container.addSingleton(IDownloadApi, downloadApi);
  container.addSingleton(IInternalBrowserApi, internalBrowserApi);
  container.addSingleton(IThreemfApi, threemf);
  container.addSingleton(IUserManageSelfApi, userAdmin);
  container.addSingleton(IShareApi, shareApi);

  if (currentUser.permissions.admin) {
    container.addSingleton(IAdminUserApi, userAdmin);
  }

  if (currentUser.id === 1) {
    panicState.inPanic = true;
    panicState.message =
      "Logged in as the local administrator user.\nThis account cannot be used for normal operation.\nPlease create a new user account in the settings and log in with that account.";
  } else {
    container.addSingleton(IUserTokenApi, user);
  }
}
