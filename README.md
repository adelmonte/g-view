# g-view

A Rust + GTK3 port of [qView](https://github.com/jurplel/qView) (7.0), the minimal image viewer.
Same behavior, same settings, same shortcuts, same translations.

## At a glance

- Install: `yay -S g-view` (AUR), or `./install.sh` (user) or `sudo PREFIX=/usr/local ./install.sh`
- Run: `g-view [file-or-folder]`
- Existing qView settings are imported automatically on first run.
- Config: `~/.config/g-view/g-view.conf`

## Building

Needs Rust and GTK 3 dev files (Arch: `gtk3`; Debian/Ubuntu: `libgtk-3-dev`), plus `lcms2`.

```
cargo build --release
```

## Migrating from qView

On first launch, if there's no g-view config yet, it looks for a qView config in:

- `~/.config/qView/qView.conf`
- `~/.var/app/com.interversehq.qView/config/qView/qView.conf` (Flatpak)
- `~/snap/qview/current/.config/qView/qView.conf` (Snap)

and imports every option, custom shortcut, recent files, last folder, window geometry and
the first-launch flag (so the welcome screen isn't shown again). qView's file is left untouched.
To re-import, delete `~/.config/g-view/g-view.conf`.

## Differences from qView

- Wayland only. The compositor places windows, so only window size (not position) is restored.
- Removed: Open URL / pasting image links, and the update checker.
- No color space setting: photos tagged with a non-sRGB color profile (e.g. iPhone Display P3) are always converted to sRGB so colors look right; everything else is untouched.
- Image decoding uses the Rust `image` crate (PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, PNM, TGA,
  QOI, HDR, EXR, DDS), `resvg` for SVG/SVGZ, and gdk-pixbuf for anything else installed.
  HEIC/AVIF/JXL work only if a gdk-pixbuf loader for them is installed.
- The "Full Screen" shortcut (F11) is restored; upstream lost it in a refactor.
- Open-with launches commands with spaces in them correctly; undo-delete also removes the
  `.trashinfo` file; rotated GIFs stay rotated while playing (upstream bugs).
- The app calls itself g-view everywhere (title bar, dialogs, translations).
- macOS/Windows-only options (hide titlebar, force dark mode, quit on last window) are kept
  in the config but hidden, exactly like qView on Linux.

Set `GVIEW_TRACE=1` to print decode/scale/draw timings.

## License

GPLv3, same as qView. Original work © 2018-2025 jurplel and qView contributors.
