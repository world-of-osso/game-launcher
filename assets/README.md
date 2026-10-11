# Windows app icon

`world-of-osso.png` is the transparent OpenAI `image_gen` source (2026-10-11): a warm bronze/gold fantasy compass crest with a broad W monogram, deliberately simple for small Windows icons.

`world-of-osso.ico` contains 16, 24, 32, 48, 64, 128 and 256 pixel images. Rebuild from the tracked source with `uv run scripts/build-icon.py` (Pillow 12.0.0). Installer, portable Start Menu shortcut and executable resource all use this ICO. Native-size light/dark previews and the desktop Start Search capture are kept with release diagnostics.
