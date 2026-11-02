# macOS app icons

These are the owner's supplied assets, not generated replacements:

- `rustfus.icon/` is the unchanged Icon Composer source supplied as `rustfus.icon` (JSON plus `Assets/rustfus.png`). It contains a red/orange gradient background and a foreground image with shadow and translucency settings. Keep the source so Xcode can compile its layered rendition.
- `rustfus.icns` is the unchanged `rustfus-mac(1).icns` supplied alongside it. Its SHA-256 is `57e0646890f4a686d9d4c2c11814ef2891e8b6d7087badd7dde7687de963170a`. It has one `ic10` (1024 px) representation, not a full hand-tuned size set. macOS scales that fallback when smaller sizes are needed.
- The source `icon.json` SHA-256 is `a95381b937141c0146c2dd82b91105f18019abaf815f681235e4c6b9a25f5519`.

The artwork has the same project licensing and Rust trademark notice as the Rustfus icon in `THIRD_PARTY_NOTICES.md`. No Apple artwork or tools are redistributed here. `res/rustfus.png` and `res/rustfus.ico` are independent cross-platform assets and are not rewritten by this packaging path.

## Build

After creating an unsigned app's `Contents/Info.plist`:

```sh
python3 packaging/macos-icons.py build/Rustfus.app
```

Default `auto` mode discovers `actool` using `xcrun`. Full Xcode 26+ and a compatible macOS host are needed for Icon Composer compilation; standalone Command Line Tools do not provide `actool`. Use `DEVELOPER_DIR` to select an installed Xcode without changing the machine-wide selection. `--actool /absolute/path/to/actool` selects a particular executable.

The script passes the `.icon` directory directly to `actool` with `--app-icon rustfus`, `--platform macosx`, `--target-device mac`, `--compile`, and `--output-partial-info-plist`. It uses the app's existing minimum deployment target (11.0 in the release workflow), rather than raising the app's minimum OS to 26.0. It installs the compiled `Assets.car` in `Contents/Resources`, merges generated icon metadata into Info.plist, and sets `CFBundleIconName` to the matching `rustfus` name. The supplied ICNS is retained as `CFBundleIconFile` even if actool generates its own flattened ICNS. Identity, version, and deployment metadata remain unchanged. The source `.icon` is not copied into the app as a substitute for compilation.

If the tool is missing or older than 26, `auto` reports fallback-only packaging, installs the supplied ICNS, and removes stale modern icon metadata. An available modern compiler failing is an error, not a silent downgrade. `--mode fallback` deliberately skips compilation; `--mode required` makes missing/old tools an error too. Release CI uses `required` on `macos-26`, so published packages cannot silently lose their layered icon. Existing `Assets.car` files are rejected rather than overwriting unrelated app resources. Run this before code signing. Use a fresh app resource directory when rebuilding.

```sh
python3 packaging/test-macos-icons.py
python3 packaging/macos-icons.py build/Rustfus.app --mode required
plutil -lint build/Rustfus.app/Contents/Info.plist
```

The tests exercise fallback, discovery failure, old tools, compilation failure, missing output, existing-catalog protection, arguments, and plist preservation with a fake compiler. They do **not** prove real actool compilation or Liquid Glass rendering. On the development host only standalone CLT was available; real compilation is enforced in release CI. Final visual verification still needs the packaged app in Finder/Dock on macOS 26+ (default/dark/clear appearances) and an older macOS for the ICNS fallback. No appearance or rendering check is claimed by the contract tests.

## References

- [Apple: Creating your app icon using Icon Composer](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer). Documents the layered source, Xcode integration, appearances, and older-release renditions. Apple's page does not document the manual command-line invocation.
- [Frank Krueger: Updating App Icons for iOS and macOS 26](https://praeclarum.org/2025/09/12/app-icons.html). Shows the actual macOS actool invocation with a `.icon` input, deployment target 11.0, generated Assets.car and ICNS, and partial Info.plist.
- [Electron Packager's Icon Composer implementation](https://github.com/electron/packager/blob/main/src/icon-composer.ts). Confirms direct `.icon` compilation, actool 26+ version checking, macOS platform/device flags, temporary output, and compiling once for a universal app. Its own minimum target is 26.0; Rustfus retains 11.0 and a separate supplied fallback.
- [Hendrik Erz: Supporting Liquid Glass Icons in Apps Without XCode](https://www.hendrik-erz.de/post/supporting-liquid-glass-icons-in-apps-without-xcode). Describes bundling compiled Assets.car for non-Xcode app builds.
