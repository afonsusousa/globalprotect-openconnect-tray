# globalprotect-openconnect-tray

> [!WARNING]
> # ⚠️ THIS IS VIBE CODED ⚠️
> **This tray app was written almost entirely by an AI coding assistant (Claude), with minimal human review.**
> It handles your VPN sign-in credentials and drives a privileged (root) service.
> It has not been audited. **Use it at your own risk. There is no warranty of any kind.** See [LICENSE](./LICENSE).

A free (GPL-3.0) tray icon and compact connection window for GlobalProtect VPNs on Linux. It's built on the open-source parts of [GlobalProtect-openconnect](https://github.com/yuezk/GlobalProtect-openconnect) by Kevin Yue: `gpservice`, `gpclient` and `gpauth`. It's an independent front end. It doesn't contain, replace or unlock upstream's proprietary GUI (`gpgui`).

## Install (openSUSE Tumbleweed)

The [`home:afonsusousa`](https://build.opensuse.org/project/show/home:afonsusousa) repository has a single package, `globalprotect-openconnect-tray`. It contains the tray and the matching `gpclient`, `gpservice` and `gpauth`, built from upstream's source for the same release, without the proprietary GUI.

Use one of these:

- **1-Click Install:** go to [software.opensuse.org/download](https://software.opensuse.org/download.html?project=home%3Aafonsusousa&package=globalprotect-openconnect-tray), choose openSUSE Tumbleweed, then click *1 Click Install*.
- **opi:** `opi globalprotect-openconnect-tray`
- **zypper:**
  ```bash
  sudo zypper ar -f https://download.opensuse.org/repositories/home:/afonsusousa/openSUSE_Tumbleweed/ home_afonsusousa
  sudo zypper --gpg-auto-import-keys in globalprotect-openconnect-tray
  ```

Then start **GlobalProtect Tray** from your app menu.

- **Replaces upstream's package:** this package ships the same `gpclient`, `gpservice` and `gpauth` files as upstream's `globalprotect-openconnect` package, so the two conflict. If you have upstream's installed, zypper offers to replace it.
- **GNOME:** install the [AppIndicator and KStatusNotifierItem Support](https://extensions.gnome.org/extension/615/appindicator-support/) extension to get the tray icon. Without it the app still works: closing the window minimizes it instead of hiding it to the tray, so you can always get back to it.

## What it does

- Uses a compact window: shield status, connection picker, server address with a Portal/Gateway toggle, and Connect/Disconnect.
- Shows a tray icon (grey/amber/green) with connect and disconnect actions.
- Saves connections with a gateway choice (blank means automatic), a sign-in method (built-in SAML window, system browser, or username and password), and HIP and TLS options. Ignoring TLS certificate errors is **on by default** for new connections; turn it off in Settings if your portal has a valid certificate.
- Remembers your sign-in, on by default. After one browser or SAML login, the tray has `gpclient` cache the portal's auth cookie for that connection (`~/.config/io.github.afonsusousa.globalprotect-openconnect-tray/sign-in/<id>.json`, mode 0600, passed as `--cookie-cache`). Later connects skip the login page until the portal expires the cookie.
- **Delete credentials** (in the ⋯ menu or Settings) clears that cookie. The next sign-in also discards the built-in sign-in window's remembered session (`--clean`), so you have to log in again.
- Settings from the app's old name, `gptray` (`~/.config/io.github.gptray`), move over automatically on first start.

## How it works (no password prompts)

The tray uses the same privileged backend as the official GUI:

1. **At startup** it runs `gpservice --desktop-credentials` through pkexec. The packaged polkit policy (`com.yuezk.gpgui.service`, `allow_active=yes`) authorizes this for the active desktop user without a password.
2. **The service** sends a one-time session credential over a private pipe. The tray uses it for the service's Noise-encrypted WebSocket, so no other program can drive the session.
3. **Sign-in** runs as you, unprivileged: `gpclient connect --cookie-only` handles the portal, SAML or password, and gateway login, then prints the gateway cookie. Passwords go to it on stdin and are never stored.
4. **The cookie** goes to the service in a `Connect` request, and the service builds the tunnel.

When you quit the tray, the service's control pipe closes, so the service disconnects the VPN and exits. That's the same lifecycle as the official GUI. Closing the window only hides it to the tray.

## Building

```bash
npm install
npm run tauri build      # binary: src-tauri/target/release/globalprotect-openconnect-tray
npm run tauri dev        # development
cd src-tauri && cargo test
```

Build dependencies: openSUSE `webkitgtk3-devel dbus-1-devel libopenssl-devel librsvg-devel`; Debian/Ubuntu `libwebkit2gtk-4.1-dev libdbus-1-dev libssl-dev librsvg2-dev build-essential`.

At runtime the tray needs upstream's `gpservice`, `gpclient` and `gpauth` from the **same release**, plus its polkit policy. The OBS package provides all of them.

The `gpservice` protocol client (`gpapi` and `common`) is a git dependency pinned to an upstream release tag in `src-tauri/Cargo.toml`:

```toml
gpapi  = { git = "https://github.com/yuezk/GlobalProtect-openconnect", tag = "v3.2.0" }
common = { git = "https://github.com/yuezk/GlobalProtect-openconnect", tag = "v3.2.0" }
```

That tag decides everything. `build.rs` reads the protocol version from it, and the OBS release builds upstream's components from the same tag. `gpservice` rejects clients built for another release.

To move to a new upstream release:
1. Change both tags.
2. Run `cargo update -p gpapi -p common`.
3. Rebuild, and fix anything the new API changed.

Run `globalprotect-openconnect-tray --minimized` to start in the tray, for example from autostart.

## Releasing on the openSUSE Build Service

```bash
sudo zypper in osc                     # once; then run `osc` once to set up credentials
packaging/obs/obs-release.sh --upload home:<your-obs-user>
```

The script produces five files in `.build/obs/` and, with `--upload`, commits them to the `globalprotect-openconnect-tray` package in that OBS project:

| File | Contents |
|---|---|
| `globalprotect-openconnect-tray-<version>.tar.gz` | This repository with the built frontend (`dist/`) and a cargo config pointing at the vendored crates |
| `vendor.tar.xz` | The tray's crates, vendored because OBS builds offline |
| `globalprotect-openconnect-<gp>.tar.gz` | Upstream at the pinned tag, cloned with submodules and packed by upstream's own `make tarball OFFLINE=1` |
| `globalprotect-openconnect-tray.spec` | Filled in from `packaging/obs/globalprotect-openconnect-tray.spec.in` |
| `globalprotect-openconnect-tray.changes` | The changelog |

The spec builds upstream with `make build OFFLINE=1 INCLUDE_GUI=0 BUILD_GUI_HELPER=0`, so the proprietary GUI is never downloaded, then builds the tray. It installs everything upstream's own package installs except the GUI's desktop entry.

The OBS project builds on plain Tumbleweed. Its repository and project config:

```xml
<repository name="openSUSE_Tumbleweed">
  <path project="openSUSE:Factory" repository="snapshot"/>
  <arch>x86_64</arch>
  <arch>aarch64</arch>
</repository>
```
```
Prefer: libayatana-appindicator3-1
```

Set them with `osc meta prj` and `osc meta prjconf`. The package needs Rust ≥ 1.91 and WebKitGTK 4.1.

Before each release, bump `version` in `src-tauri/Cargo.toml` and add an entry to `packaging/obs/globalprotect-openconnect-tray.changes`, for example with `osc vc`.

## License

The tray is GPL-3.0; see [LICENSE](./LICENSE). The OBS package also contains upstream's components, under their own licenses: GPL-3.0 (`gpclient`, `gpservice`, `gpauth`), MIT (`gpapi`, `common` and other crates) and LGPL-2.1 (OpenConnect). Their license files are installed with the package.
