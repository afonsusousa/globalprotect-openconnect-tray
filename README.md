# gptray

> [!WARNING]
> # ⚠️ THIS IS VIBE CODED ⚠️
> **gptray was written almost entirely by an AI coding assistant (Claude), with minimal human review.**
> It handles your VPN sign-in credentials and drives a privileged (root) service.
> It has not been audited. **Use it at your own risk. There is no warranty of any kind.** See [LICENSE](./LICENSE).

## Install (openSUSE Tumbleweed)

The [`home:afonsusousa`](https://build.opensuse.org/project/show/home:afonsusousa) repository has everything you need: gptray and the matching `globalprotect-openconnect`, the upstream packages unchanged.

Use one of these:

- **1-Click Install:** go to [software.opensuse.org/download](https://software.opensuse.org/download.html?project=home%3Aafonsusousa&package=gptray), choose openSUSE Tumbleweed, then click *1 Click Install*.
- **opi:** `opi gptray`
- **zypper:**
  ```bash
  sudo zypper ar -f https://download.opensuse.org/repositories/home:/afonsusousa/openSUSE_Tumbleweed/ home_afonsusousa
  sudo zypper --gpg-auto-import-keys in gptray
  ```

Then start **GlobalProtect (gptray)** from your app menu.

**GNOME users:** install the [AppIndicator and KStatusNotifierItem Support](https://extensions.gnome.org/extension/615/appindicator-support/) extension to get the tray icon. Without it, gptray still works: closing the window minimizes it instead of hiding it to the tray, so you can always get back to it.

If you already have upstream's `home:yuezk` repository, keep it or remove it. Both carry the same `globalprotect-openconnect`.

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

gptray is built against `gpapi` from a pinned upstream release tag, and the service only accepts clients built for its exact release. Keep that tag in step with your installed `gpservice`; see *Building*.

## Requirements

- GlobalProtect-openconnect installed: `gpservice`, `gpclient`, `gpauth`, and its polkit policy.
- WebKitGTK 4.1 and a tray implementation. On GNOME, install the *AppIndicator and KStatusNotifierItem Support* extension for the tray icon. Without it, closing the window minimizes instead of hiding.

## Building

```bash
cd gptray   # or apps/gptray in the GlobalProtect-openconnect fork
npm install
npm run tauri build      # binary: src-tauri/target/release/gptray
npm run tauri dev        # development
cd src-tauri && cargo test
```

Build dependencies: openSUSE `webkitgtk3-devel dbus-1-devel libopenssl-devel librsvg-devel`; Debian/Ubuntu `libwebkit2gtk-4.1-dev libdbus-1-dev libssl-dev librsvg2-dev build-essential`.

**This folder is standalone.** Copy it anywhere, even outside this repository, and it builds. The `gpservice` protocol client (`gpapi` and `common`) is a git dependency pinned to an upstream release tag in `src-tauri/Cargo.toml`:

```toml
gpapi  = { git = "https://github.com/yuezk/GlobalProtect-openconnect", tag = "v3.2.0" }
common = { git = "https://github.com/yuezk/GlobalProtect-openconnect", tag = "v3.2.0" }
```

That tag must match the installed `globalprotect-openconnect`, because `gpservice` rejects clients built for another release. `build.rs` and the OBS spec read the version from it. To target a new upstream release, change both tags, run `cargo update -p gpapi -p common`, rebuild, and fix anything the new API changed.

Run `gptray --minimized` to start in the tray, for example from autostart.

## Releasing on the openSUSE Build Service

```bash
sudo zypper in osc                     # once; then run `osc` once to set up credentials
packaging/obs/obs-release.sh --upload home:<your-obs-user>
```

The script builds the frontend, then packs the gptray folder with the built `dist/` into a source tarball. It vendors every crate, including the pinned `gpapi` and `common`, into `vendor.tar.xz`, because OBS builds offline. It ships the matching `src-tauri/.cargo/config.toml` and fills in `gptray.spec` from `packaging/obs/gptray.spec.in`. With `--upload` it also creates or updates the `gptray` package in that OBS project and commits. Leave out `--upload` to only produce the files in `.build/obs/`. It needs nothing outside this folder.

Set up the OBS project to build on top of upstream's, so `globalprotect-openconnect` comes from `home:yuezk`. The live project `home:afonsusousa` uses this repository and project config:

```xml
<repository name="openSUSE_Tumbleweed">
  <path project="home:yuezk" repository="openSUSE_Tumbleweed"/>
  <arch>x86_64</arch>
  <arch>aarch64</arch>
</repository>
```
```
Prefer: libayatana-appindicator3-1
```

Use `osc meta prj` and `osc meta prjconf` to set them. The project also *aggregates* upstream's built package with `osc aggregatepac home:yuezk globalprotect-openconnect home:afonsusousa`, so users only need this one repository. The `Prefer` line resolves upstream's "appindicator or ayatana" dependency for OBS. The package needs Rust ≥ 1.91 and WebKitGTK 4.1. It requires exactly the matching `globalprotect-openconnect` version, because `gpservice` rejects clients built for another release. Users also need that package, for example from its own OBS project.

Before each release, bump `version` in `src-tauri/Cargo.toml` and add an entry to `packaging/obs/gptray.changes`, for example with `osc vc`.

## License

GPL-3.0, the same as `gpclient`. See [LICENSE](./LICENSE).
