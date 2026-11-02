mod profile;
mod vpn;

use std::sync::Arc;

use tauri::{
  AppHandle, Emitter, Manager, State, WindowEvent,
  image::Image,
  menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
  tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use profile::{Profile, ProfileStore};
use vpn::{Hooks, LogLine, Status, Vpn};

const TRAY_ID: &str = "main";
const ICON_CONNECTED: &[u8] = include_bytes!("../icons/tray-connected.png");
const ICON_CONNECTING: &[u8] = include_bytes!("../icons/tray-connecting.png");
const ICON_DISCONNECTED: &[u8] = include_bytes!("../icons/tray-disconnected.png");

struct AppState {
  vpn: Arc<Vpn>,
  profiles: ProfileStore,
}

type CmdResult<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
  format!("{e:#}")
}

#[tauri::command]
fn list_profiles(state: State<AppState>) -> Vec<Profile> {
  state.profiles.load()
}

#[tauri::command]
fn save_profile(app: AppHandle, state: State<AppState>, profile: Profile) -> CmdResult<Vec<Profile>> {
  let profiles = state.profiles.save(profile).map_err(err)?;
  refresh_tray(&app);
  Ok(profiles)
}

#[tauri::command]
fn delete_profile(app: AppHandle, state: State<AppState>, id: String) -> CmdResult<Vec<Profile>> {
  let profiles = state.profiles.delete(&id).map_err(err)?;
  refresh_tray(&app);
  Ok(profiles)
}

#[tauri::command]
async fn connect(state: State<'_, AppState>, id: String, password: Option<String>) -> CmdResult<()> {
  let profile = state.profiles.get(&id).ok_or("Profile not found")?;
  let cache = state.profiles.sign_in_cache(&profile);
  let clean = state.profiles.take_clean(&profile.id);
  state.vpn.clone().connect(profile, password, cache, clean).await.map_err(err)
}

#[tauri::command]
fn has_saved_sign_in(state: State<AppState>, id: String) -> bool {
  state.profiles.has_saved_sign_in(&id)
}

#[tauri::command]
fn forget_sign_in(state: State<AppState>, id: String) -> CmdResult<()> {
  state.profiles.forget_sign_in(&id).map_err(err)
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>) -> CmdResult<()> {
  state.vpn.clone().disconnect().await.map_err(err)
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Status {
  state.vpn.status()
}

#[tauri::command]
fn get_logs(state: State<AppState>) -> Vec<LogLine> {
  state.vpn.logs()
}

#[tauri::command]
fn clear_logs(state: State<AppState>) {
  state.vpn.clear_logs()
}

/// Quitting closes gpservice's control pipe, which disconnects the VPN.
#[tauri::command]
fn quit(app: AppHandle) {
  app.exit(0)
}

/// True when a StatusNotifier host (the tray) is running. Stock GNOME has none
/// unless the AppIndicator extension is enabled, so a hidden window would be lost.
fn tray_available() -> bool {
  use dbus::blocking::Connection;
  let Ok(conn) = Connection::new_session() else {
    return false;
  };
  let proxy = conn.with_proxy(
    "org.freedesktop.DBus",
    "/org/freedesktop/DBus",
    std::time::Duration::from_millis(500),
  );
  proxy
    .method_call("org.freedesktop.DBus", "NameHasOwner", ("org.kde.StatusNotifierWatcher",))
    .map(|(owned,): (bool,)| owned)
    .unwrap_or(false)
}

/// Put the window away without quitting (which would disconnect the VPN):
/// into the tray when there is one, otherwise minimized to the dock/taskbar.
fn put_away(window: &tauri::WebviewWindow) {
  if tray_available() {
    let _ = window.hide();
  } else {
    let _ = window.minimize();
  }
}

#[tauri::command]
fn close_window(window: tauri::WebviewWindow) {
  put_away(&window);
}

fn show_window(app: &AppHandle) {
  if let Some(window) = app.get_webview_window("main") {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
  }
}

fn icon_for(status: &Status) -> &'static [u8] {
  match status {
    Status::Connected { .. } => ICON_CONNECTED,
    Status::Connecting { .. } | Status::Disconnecting => ICON_CONNECTING,
    Status::Disconnected | Status::Error { .. } => ICON_DISCONNECTED,
  }
}

fn build_menu(app: &AppHandle, status: &Status, profiles: &[Profile]) -> tauri::Result<Menu<tauri::Wry>> {
  let menu = Menu::new(app)?;
  let mut label = status.label();
  if label.chars().count() > 60 {
    label = label.chars().take(57).collect::<String>() + "…";
  }
  menu.append(&MenuItem::with_id(app, "status", label, false, None::<&str>)?)?;
  menu.append(&PredefinedMenuItem::separator(app)?)?;

  let idle = matches!(status, Status::Disconnected | Status::Error { .. });
  let connect = Submenu::with_id(app, "connect", "Connect", idle && !profiles.is_empty())?;
  for p in profiles {
    connect.append(&MenuItem::with_id(app, format!("connect:{}", p.id), &p.name, true, None::<&str>)?)?;
  }
  menu.append(&connect)?;

  let (text, enabled) = match status {
    Status::Connecting { .. } => ("Cancel", true),
    Status::Connected { .. } => ("Disconnect", true),
    _ => ("Disconnect", false),
  };
  menu.append(&MenuItem::with_id(app, "disconnect", text, enabled, None::<&str>)?)?;
  menu.append(&PredefinedMenuItem::separator(app)?)?;
  menu.append(&MenuItem::with_id(app, "show", "Open window", true, None::<&str>)?)?;
  menu.append(&MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?)?;
  Ok(menu)
}

fn refresh_tray(app: &AppHandle) {
  let handle = app.clone();
  let _ = app.run_on_main_thread(move || {
    let state = handle.state::<AppState>();
    let status = state.vpn.status();
    let Some(tray) = handle.tray_by_id(TRAY_ID) else {
      return;
    };
    let result = (|| -> tauri::Result<()> {
      tray.set_menu(Some(build_menu(&handle, &status, &state.profiles.load())?))?;
      tray.set_tooltip(Some(status.label()))?;
      tray.set_icon(Some(Image::from_bytes(icon_for(&status))?))?;
      Ok(())
    })();
    if let Err(e) = result {
      log::warn!("Cannot update tray: {e}");
    }
  });
}

fn on_menu(app: &AppHandle, id: &str) {
  let state = app.state::<AppState>();
  match id {
    "show" => show_window(app),
    "quit" => app.exit(0),
    "disconnect" => {
      let vpn = state.vpn.clone();
      tauri::async_runtime::spawn(async move {
        if let Err(e) = vpn.clone().disconnect().await {
          vpn.log("ERROR", format!("{e:#}"));
        }
      });
    }
    _ => {
      let Some(profile) = id.strip_prefix("connect:").and_then(|id| state.profiles.get(id)) else {
        return;
      };
      if profile.needs_password() {
        // Passwords are only ever typed into our own window.
        show_window(app);
        let _ = app.emit("request-password", profile.id);
        return;
      }
      let vpn = state.vpn.clone();
      let cache = state.profiles.sign_in_cache(&profile);
      let clean = state.profiles.take_clean(&profile.id);
      tauri::async_runtime::spawn(async move {
        if let Err(e) = vpn.clone().connect(profile, None, cache, clean).await {
          vpn.log("ERROR", format!("{e:#}"));
        }
      });
    }
  }
}

fn main() {
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
  let start_minimized = std::env::args().any(|a| a == "--minimized");

  tauri::Builder::default()
    .setup(move |app| {
      let config_dir = app.path().app_config_dir()?;
      let status_handle = app.handle().clone();
      let log_handle = app.handle().clone();
      let hooks = Hooks {
        on_status: Box::new(move |status| {
          let _ = status_handle.emit("status", status);
          refresh_tray(&status_handle);
        }),
        on_log: Box::new(move |line| {
          let _ = log_handle.emit("log", line);
        }),
      };
      let vpn = Arc::new(Vpn::new(hooks));
      vpn.log("WARN", "VIBE CODED: written almost entirely by an AI coding assistant (Claude) with minimal human review. It handles VPN sign-in credentials and drives a root service. Use at your own risk. No warranty.");
      app.manage(AppState {
        vpn: vpn.clone(),
        profiles: ProfileStore::new(config_dir),
      });

      TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_DISCONNECTED)?)
        .tooltip("GlobalProtect VPN")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
          if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
          } = event
          {
            show_window(tray.app_handle());
          }
        })
        .build(app)?;

      refresh_tray(app.handle());
      // Like the official GUI: the privileged service lives as long as this app.
      let service = vpn.clone();
      tauri::async_runtime::spawn(async move { service.start_service() });

      if start_minimized && let Some(window) = app.get_webview_window("main") {
        put_away(&window);
      }
      Ok(())
    })
    .on_window_event(|window, event| {
      // Closing keeps the app (and the VPN) running; see `put_away`.
      if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Some(window) = window.app_handle().get_webview_window(window.label()) {
          put_away(&window);
        }
      }
    })
    .invoke_handler(tauri::generate_handler![
      list_profiles,
      save_profile,
      delete_profile,
      connect,
      disconnect,
      get_status,
      get_logs,
      clear_logs,
      close_window,
      has_saved_sign_in,
      forget_sign_in,
      quit
    ])
    .run(tauri::generate_context!())
    .expect("error while running gptray");
}
