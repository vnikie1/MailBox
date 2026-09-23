# Brand assets

The app icon and brand marks, as the designer delivered them on 2026-09-17. This folder is
the source; everything the app ships is generated from it by `npm run icon`
(`tools/build-icons.ps1`). Do not edit the generated copies — edit here and rebuild.

## What is here

| Folder   | Contents                                                                                                              |
| -------- | --------------------------------------------------------------------------------------------------------------------- |
| `store/` | `halcyon-icon-accent.svg`, the app icon master; the bare mark; lockups and marks for listings; the 300px Store icon. |
| `ico/`   | The Win32 icon at 16, 20, 24, 32, 40, 48, 64 and 256 px, each drawn for its size.                                     |
| `msix/`  | The Store package set, already named with the `scale-`, `targetsize-` and `altform-` qualifiers MSIX resolves.        |

## The rules the designer set

- **Accent `#EC3013`, full bleed.** The plated icons are the white envelope on red, edge to edge.
  The package manifest's `BackgroundColor` is the same red, so a tile never shows a plate edge.
- **Neutral plate `#F3F2F2`** where the icon sits on a light surface of our own — the installer's
  welcome page.
- **The small sizes are drawn, not scaled.** The envelope's stroke is 8% of the width at 256 px
  and 12% at 16 px, so it keeps its weight as it shrinks. A downscaled 256 thins to a hairline;
  that is why `icon.ico`, `32x32.png`, `64x64.png` and the tray icons come from `ico/` rather
  than from the SVG.
- **Unplated** (`altform-unplated`) is the white glyph on transparency, for the dark taskbar;
  **light unplated** (`altform-lightunplated`) is the ink glyph, for the light one.

## What `npm run icon` writes

| Output                         | From                                  | Used for                                                 |
| ------------------------------ | ------------------------------------- | -------------------------------------------------------- |
| `src-tauri/icons/icon.ico`     | `ico/` — PNG entries, 32 px first     | The exe, the window and the taskbar (Tauri takes entry 0) |
| `src-tauri/icons/tray/`        | `ico/` 16–48                          | The notification area, chosen by display scale           |
| `src-tauri/icons/*.png`        | the SVG, via `tauri icon`             | Bundle icons and the legacy `Square*Logo` set            |
| `src-tauri/msix/Assets/`       | `msix/`, copied as named              | The Store package                                        |
| `src-tauri/installer/*.bmp`    | `msix/Square44x44Logo.scale-200.png`, `ico/halcyon-40.png` | The NSIS header (150×57) and welcome page (164×314) |
| `public/favicon.svg`           | the SVG                               | The browser build's tab                                  |

## What is deliberately not used

- **`msix/SplashScreen.*`.** The designer's manifest snippet declares a splash screen; ours does
  not. Windows never shows one for a full-trust desktop app, and declaring one failed the App
  Certification Kit's resource test once already (see the comment in
  `src-tauri/msix/AppxManifest.xml`). The images stay here in case that changes.
- **The C2PA `caBX` chunk** in every PNG is left intact. It is provenance metadata, ignored by
  every decoder that matters, and stripping it would alter files that are the designer's to sign.

The designer's full export, with its own preview page and tooling, is not tracked
(`Halcyon Mail App Logo/` in `.gitignore`).
