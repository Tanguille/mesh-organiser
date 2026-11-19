import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { IUserLoginApi, IUserLogoutApi } from "../shared/user_api";

export class WebUserLoginApi implements IUserLoginApi, IUserLogoutApi {
  private requestApi: IServerRequestApi;

  constructor(requestApi: IServerRequestApi) {
    this.requestApi = requestApi;
  }

  async loginUser(email: string, password: string): Promise<void> {
    await this.requestApi.request<void>("/login/password", HttpMethod.POST, {
      email: email,
      password: password,
      group_deletion_protocol: 1,
    });
  }

  async logoutCurrentUser(): Promise<void> {
    await this.requestApi.request<void>("/logout", HttpMethod.POST);
  }
}
