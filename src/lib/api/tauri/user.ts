import { invoke } from "@tauri-apps/api/core";
import { parseRawUser, type RawUser } from "../shared/raw_model";
import {
  IAdminUserApi,
  ISwitchUserApi,
  IUserManageSelfApi,
  newlyCreatedUser,
  type IUserApi,
  type User,
} from "../shared/user_api";

export class UserApi
  implements IUserApi, IAdminUserApi, ISwitchUserApi, IUserManageSelfApi
{
  async isAuthenticated(): Promise<boolean> {
    return true;
  }

  async getCurrentUser(): Promise<User> {
    const user = await invoke<RawUser>("get_current_user", {});

    user.permissions.push("Admin");

    return parseRawUser(user);
  }

  async getAvailableUsers(): Promise<User[]> {
    const users = await invoke<RawUser[]>("get_users", {});
    console.log("get users", users);

    return users.map((user) => parseRawUser(user));
  }

  async getAllUsers(): Promise<User[]> {
    return await this.getAvailableUsers();
  }

  async addUser(
    username: string,
    email: string,
    password: string,
  ): Promise<User> {
    const id = await invoke<number>("add_user", {
      userName: username,
      userEmail: email,
      userPassword: password,
    });

    return newlyCreatedUser(id, username, email);
  }

  async deleteUser(user: User): Promise<void> {
    await invoke("delete_user", { userId: user.id });
  }

  async switchUser(user: User): Promise<void> {
    await invoke("set_current_user", { userId: user.id });
  }

  async editUser(user: User): Promise<void> {
    await invoke("edit_user", {
      userId: user.id,
      userName: user.username,
      userEmail: user.email,
      userLastSync: user.lastSync?.toISOString() ?? null,
      userSyncToken: user.syncToken,
      userSyncUrl: user.syncUrl,
    });
  }

  async editUserPassword(_user: User, _newPassword: string): Promise<void> {
    throw new Error("Method not implemented.");
  }

  async editSelf(user: User): Promise<void> {
    await this.editUser(user);
  }

  async editSelfPassword(_newPassword: string): Promise<void> {
    throw new Error("Method not implemented.");
  }
}
