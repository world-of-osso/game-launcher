# /// script
# requires-python = ">=3.12"
# dependencies = ["pillow==12.0.0"]
# ///
"""Build the Windows icon from the tracked OpenAI-generated source PNG."""

from pathlib import Path
from PIL import Image

SIZES = (16, 24, 32, 48, 64, 128, 256)
ROOT = Path(__file__).resolve().parents[1]


def main():
    source = Image.open(ROOT / 'assets/world-of-osso.png').convert('RGBA')
    if source.width != source.height:
        raise ValueError('App icon source must be square')
    output = ROOT / 'assets/world-of-osso.ico'
    source.save(output, format='ICO', sizes=[(size, size) for size in SIZES])
    actual = Image.open(output).ico.sizes()
    expected = {(size, size) for size in SIZES}
    if actual != expected:
        raise ValueError(f'Icon sizes {actual}, expected {expected}')
    print(f'Built {output}: {", ".join(map(str, SIZES))} px')


if __name__ == '__main__':
    main()
