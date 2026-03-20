import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import {
  createUserInstance,
  IUserManageSelfApi,
  permissionsToStringArray,
  type IAdminUserApi,
  type User,
} from "../shared/user_api";
import { parseTauriRawUser, type TauriRawUser } from "../tauri/user";

export class WebUserAdminApi implements IAdminUserApi, IUserManageSelfApi {
  private requestApi: IServerRequestApi;
  private currentUser: User;

  constructor(requestApi: IServerRequestApi, currentUser: User) {
    this.requestApi = requestApi;
    this.currentUser = currentUser;
  }

  async getAllUsers(): Promise<User[]> {
    const users = await this.requestApi.request<TauriRawUser[]>(
      "/users",
      HttpMethod.GET,
    );
    return users.map((user) => parseTauriRawUser(user));
  }

  async addUser(
    username: string,
    email: string,
    password: string,
  ): Promise<User> {
    const data = {
      user_name: username,
      user_email: email,
      user_password: password,
    };

    const userId = (
      await this.requestApi.request<{ id: number }>(
        "/users",
        HttpMethod.POST,
        data,
      )
    ).id;
    return createUserInstance(
      userId,
      username,
      email,
      new Date().toISOString(),
      [],
      null,
      null,
      null,
    );
  }

  async deleteUser(user: User): Promise<void> {
    await this.requestApi.request<void>(`/users/${user.id}`, HttpMethod.DELETE);
  }

  async editUser(user: User): Promise<void> {
    const dataUserEdit = {
      user_name: user.username,
      user_email: user.email,
    };

    await this.requestApi.request<void>(
      `/users/${user.id}`,
      HttpMethod.PUT,
      dataUserEdit,
    );

    const dataPermissionsEdit = {
      permissions: permissionsToStringArray(user.permissions),
    };

    await this.requestApi.request<void>(
      `/users/${user.id}/permissions`,
      HttpMethod.PUT,
      dataPermissionsEdit,
    );
  }

  async editUserPassword(user: User, newPassword: string): Promise<void> {
    const data = {
      new_password: newPassword,
    };

    await this.requestApi.request<void>(
      `/users/${user.id}/password`,
      HttpMethod.PUT,
      data,
    );
  }

  async editSelf(user: User): Promise<void> {
    const dataUserEdit = {
      user_name: user.username,
      user_email: user.email,
    };

    await this.requestApi.request<void>(
      `/users/${user.id}`,
      HttpMethod.PUT,
      dataUserEdit,
    );
  }

  async editSelfPassword(newPassword: string): Promise<void> {
    const data = {
      new_password: newPassword,
    };

    await this.requestApi.request<void>(
      `/users/${this.currentUser.id}/password`,
      HttpMethod.PUT,
      data,
    );
  }
}
