import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import {
  IUserManageSelfApi,
  newlyCreatedUser,
  permissionsToStringArray,
  type IAdminUserApi,
  type User,
} from "../shared/user_api";
import { parseRawUser, type RawUser } from "../shared/raw_model";

export class WebUserAdminApi implements IAdminUserApi, IUserManageSelfApi {
  private requestApi: IServerRequestApi;
  private currentUser: User;

  constructor(requestApi: IServerRequestApi, currentUser: User) {
    this.requestApi = requestApi;
    this.currentUser = currentUser;
  }

  async getAllUsers(): Promise<User[]> {
    const users = await this.requestApi.request<RawUser[]>(
      "/users",
      HttpMethod.GET,
    );
    return users.map((user) => parseRawUser(user));
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
    return newlyCreatedUser(userId, username, email);
  }

  async deleteUser(user: User): Promise<void> {
    await this.requestApi.request<void>(`/users/${user.id}`, HttpMethod.DELETE);
  }

  async editUser(user: User): Promise<void> {
    await this.editNameAndEmail(user.id, user);

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
    // Targets the user this API was built for, not `user.id`: the settings card
    // passes the global current user, which in desktop online-account mode is
    // the local user with a local id.
    await this.editNameAndEmail(this.currentUser.id, user);
  }

  private async editNameAndEmail(userId: number, user: User): Promise<void> {
    const dataUserEdit = {
      user_name: user.username,
      user_email: user.email,
    };

    await this.requestApi.request<void>(
      `/users/${userId}`,
      HttpMethod.PUT,
      dataUserEdit,
    );
  }

  async editSelfPassword(newPassword: string): Promise<void> {
    await this.editUserPassword(this.currentUser, newPassword);
  }
}
