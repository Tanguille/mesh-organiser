import { newlyCreatedUser, type IUserApi, type User } from "../shared/user_api";

export class DemoUserApi implements IUserApi {
  async isAuthenticated(): Promise<boolean> {
    return true;
  }

  async getCurrentUser(): Promise<User> {
    return newlyCreatedUser(1, "Demo User", "demo@user.com");
  }
}
