import { invoke } from "@tauri-apps/api/core";

export type AuthMethod = "embedded" | "browser" | "password";

export interface Profile {
  id: string;
  name: string;
  portal: string;
  gateway: string;
  asGateway: boolean;
  auth: AuthMethod;
  browser: string;
  username: string;
  hip: boolean;
  rememberSignIn: boolean;
  ignoreTlsErrors: boolean;
}

export type Status =
  | { state: "disconnected" }
  | { state: "connecting"; profileId: string; profileName: string }
  | { state: "connected"; profileId: string | null; profileName: string | null; since: number }
  | { state: "disconnecting" }
  | { state: "error"; message: string };

export interface LogLine {
  time: number;
  level: string;
  message: string;
}

export const emptyProfile = (): Profile => ({
  id: "",
  name: "",
  portal: "",
  gateway: "",
  asGateway: false,
  auth: "embedded",
  browser: "",
  username: "",
  hip: false,
  rememberSignIn: true,
  ignoreTlsErrors: true,
});

export const api = {
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  saveProfile: (profile: Profile) => invoke<Profile[]>("save_profile", { profile }),
  deleteProfile: (id: string) => invoke<Profile[]>("delete_profile", { id }),
  connect: (id: string, password?: string) => invoke<void>("connect", { id, password: password ?? null }),
  disconnect: () => invoke<void>("disconnect"),
  getStatus: () => invoke<Status>("get_status"),
  getLogs: () => invoke<LogLine[]>("get_logs"),
  clearLogs: () => invoke<void>("clear_logs"),
  quit: () => invoke<void>("quit"),
  closeWindow: () => invoke<void>("close_window"),
  hasSavedSignIn: (id: string) => invoke<boolean>("has_saved_sign_in", { id }),
  forgetSignIn: (id: string) => invoke<void>("forget_sign_in", { id }),
};
