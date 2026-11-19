import { expect, it, vi } from "vitest";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { WebUserLoginApi } from "./login";

it("advertises group deletion protocol when logging in with a password", async () => {
  const request = vi.fn().mockResolvedValue(undefined);
  const api = new WebUserLoginApi({ request } as unknown as IServerRequestApi);
  await api.loginUser("alice@example.test", "test-password");
  expect(request).toHaveBeenCalledExactlyOnceWith(
    "/login/password",
    HttpMethod.POST,
    {
      email: "alice@example.test",
      password: "test-password",
      group_deletion_protocol: 1,
    },
  );
});
