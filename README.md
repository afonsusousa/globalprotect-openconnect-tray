# globalprotect-openconnect-tray

> [!WARNING]
> Largely vibe coded (AI-written, not audited). Use at your own risk.

A free tray app for GlobalProtect VPNs on Linux, built on the open-source parts of [GlobalProtect-openconnect](https://github.com/yuezk/GlobalProtect-openconnect). No password prompts, remembered SAML sign-in, no proprietary GUI.

> [!IMPORTANT]
> Made for **Universidade do Porto (U.Porto)** users, whose VPN uses SAML sign-in. **Only SAML sign-in is supported.** Not implemented: one-time codes (MFA/OTP), client certificates, password sign-in at the gateway after SAML at the portal, and manual SAML. Other GlobalProtect VPNs may work if they use plain SAML.

**Install:** [1-Click / instructions](https://software.opensuse.org/download.html?project=home%3Aafonsusousa&package=globalprotect-openconnect-tray) for openSUSE Tumbleweed, Fedora 43/44/45/Rawhide and Ubuntu 26.04/26.10, or `opi globalprotect-openconnect-tray`.

**Build:** `npm install && npm run tauri build`. Release to OBS: `packaging/obs/obs-release.sh --upload home:<user>`.

GPL-3.0. Upstream components keep their own licenses.
