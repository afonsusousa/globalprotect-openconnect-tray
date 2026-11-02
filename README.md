# gptray

A small, free (GPL-3.0) tray app and connection window for GlobalProtect-openconnect on Linux.
It's an independent front end. It doesn't replace or unlock the proprietary `gpgui`. It drives the open-source `gpservice` and `gpclient`.

## What it does

- Uses a compact window: shield status, connection picker, server address with a Portal/Gateway toggle, and Connect/Disconnect.
- Shows a tray icon (grey/amber/green) with connect and disconnect actions.
- Saves connections with a gateway choice (blank means automatic), a sign-in method (built-in SAML window, system browser, or username and password), and HIP and TLS options. Ignoring TLS certificate errors is **on by default** for new connections; turn it off in Settings if your portal has a valid certificate.
- Remembers your sign-in, on by default. After one browser or SAML login, gptray has `gpclient` cache the portal's auth cookie for that connection (`~/.config/io.github.gptray/sign-in/<id>.json`, mode 0600, passed as `--cookie-cache`). Later connects skip the login page until the portal expires the cookie. Use **Delete credentials** (in the ⋯ menu or Settings) to clear it. The next sign-in also discards the built-in sign-in window's remembered session (`--clean`), so you have to log in again.

## How it works (no password prompts)

gptray uses the same privileged backend as the official GUI:

1. **At startup** it runs `gpservice --desktop-credentials` through pkexec. The polkit policy that GlobalProtect-openconnect installs (`com.yuezk.gpgui.service`, `allow_active=yes`) authorizes this for the active desktop user without a password.
2. **The service** sends a one-time session credential over a private pipe. gptray uses it for the service's Noise-encrypted WebSocket, so no other program can drive the session.
3. **Sign-in** runs as you, unprivileged: `gpclient connect --cookie-only` handles the portal, SAML or password, and gateway login, then prints the gateway cookie. Passwords go to it on stdin and are never stored.
4. **The cookie** goes to the service in a `Connect` request, and the service builds the tunnel.

When you quit gptray, the service's control pipe closes, so the service disconnects the VPN and exits. That's the same lifecycle as the official GUI. Closing the window only hides it to the tray.

gptray is built against this repository's `gpapi`, and the service only accepts clients built for its exact release. Rebuild gptray from the tag that matches your installed `gpservice`.

## Requirements

- GlobalProtect-openconnect installed: `gpservice`, `gpclient`, `gpauth`, and its polkit policy.
- WebKitGTK 4.1 and a tray implementation. On GNOME, install the *AppIndicator and KStatusNotifierItem Support* extension.

## Building

```bash
cd apps/gptray
npm install
npm run tauri build      # binary: src-tauri/target/release/gptray
npm run tauri dev        # development
cd src-tauri && cargo test
```

Build dependencies: openSUSE `webkitgtk3-devel dbus-1-devel libopenssl-devel librsvg-devel`; Debian/Ubuntu `libwebkit2gtk-4.1-dev libdbus-1-dev libssl-dev librsvg2-dev build-essential`.

This crate has its own Cargo workspace, so it builds separately from the main repository workspace.

Run `gptray --minimized` to start in the tray, for example from autostart.

## License

GPL-3.0, the same as `gpclient`. See [LICENSE](./LICENSE).
