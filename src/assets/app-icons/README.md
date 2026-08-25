# Application icon provenance

These icons were copied unchanged from the locally installed vendor `.deb` packages on 2026-08-24.

| Asset | Installed package path | SHA-256 |
| --- | --- | --- |
| `vscode.png` | `/usr/share/pixmaps/vscode.png` | `7537330cec94b308feaa9bb66db45b5554b8379ec7dce83990521d2860bca4b2` |
| `google-chrome.png` | `/opt/google/chrome/product_logo_256.png` | `e2e575b9d88afb081caf9276886854adb60b605da713ecef805f621f8b4f0767` |
| `chatgpt.png` | `/usr/share/pixmaps/chatgpt.png` | `cfe031774dd6aabdceca35338f329bc4844592767d043b7b4a56d8fb097dffd0` |
| `flclash.png` | `/usr/share/icons/hicolor/256x256/apps/FlClash.png` | `b6045af66e2e765643a50ac4871d388a9004e90dea93046696ac742ff8bf2e23` |
| `wechat.png` | `/usr/share/icons/hicolor/256x256/apps/wechat.png` | `9381f14469bd3dcb67c842384a47ea220790c601870e71393ded1e943e46f1f4` |
| `wemeet.svg` | `/opt/wemeet/wemeet.svg` | `8c7df6f803e60c37bf366032622f07a5b099b23c5c5c48795b79cf09d6d637dd` |

They are used only to identify the corresponding installed applications in UManager.

## Development-environment toolchain icons

Node.js and Rust are installed via user-level version managers (nvm / rustup), not `.deb`
packages, so there is no bundled vendor logo to copy. Their icons are the standard Simple
Icons glyphs, fetched on 2026-08-25 from `https://cdn.simpleicons.org/` and colored to match
each toolchain's accent color:

| Asset | Source (Simple Icons) | Color |
| --- | --- | --- |
| `nodejs.svg` | `nodedotjs` | `#5fa04e` |
| `rust.svg` | `rust` | `#c0562a` |
