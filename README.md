# globalprotect-openconnect-tray

> [!WARNING]
> Largely vibe coded (AI-written, not audited). Use at your own risk.

A free tray app for GlobalProtect VPNs on Linux, built on the open-source parts of [GlobalProtect-openconnect](https://github.com/yuezk/GlobalProtect-openconnect). No password prompts, remembered SAML sign-in, no proprietary GUI.

**Install:** [1-Click / instructions](https://software.opensuse.org/download.html?project=home%3Aafonsusousa&package=globalprotect-openconnect-tray) for openSUSE Tumbleweed and Ubuntu 26.04/26.10, or `opi globalprotect-openconnect-tray`.

**Build:** `npm install && npm run tauri build`. Release to OBS: `packaging/obs/obs-release.sh --upload home:<user>`.

GPL-3.0. Upstream components keep their own licenses.
