# Audio Bartender logo

Three bottles on the back bar, their heights reading like a mixer's level meters.

## Files

| File | Use |
|---|---|
| `icon.svg` | App icon, 40 px and up |
| `icon-small.svg` | App icon, 32 px and below: chunkier bottles that stay apart |
| `icon-flat.svg` | App icon without the gradient, for print and one-colour work |
| `glyph.svg`, `glyph-small.svg` | The bottles alone in `currentColor`, for UI on an accent tile |
| `glyph-black.svg`, `glyph-white.svg`, `glyph-cyan.svg` | The bottles alone in one colour |
| `lockup-horizontal.svg`, `lockup-horizontal-white.svg` | Icon and name side by side, on light and dark |
| `lockup-stacked.svg` | Icon above the name |
| `wordmark.svg` | The name alone |

`scripts/make-icon.ps1` draws `assets/icon.ico` and `assets/icon.png` from the same geometry, and `scripts/make-wizard-images.ps1` draws the installer art from `assets/icon.png`. Run both after changing the mark.

## Colour

| Name | HEX | RGB | Use |
|---|---|---|---|
| Cyan | `#06B6D4` | 6 182 212 | Tile gradient start |
| Deep teal | `#0E7490` | 14 116 144 | Tile gradient end |
| Teal | `#0891B2` | 8 145 178 | Flat tile, one-colour glyph |
| Ink | `#1C1B22` | 28 27 34 | Wordmark on light |
| White | `#FFFFFF` | 255 255 255 | Bottles, wordmark on dark |

Inside the app the corner tile takes the user's accent colour instead, so the mark always matches the flyout.

## Type

The wordmark is Inter SemiBold at its display optical size, tracked −1 %, converted to outlines. Inter uses the SIL Open Font License, which allows logo use.

## Clear space and sizes

- **Clear space:** keep a margin of a quarter of the tile's width on every side.
- **Smallest icon:** 16 px. Use `icon-small.svg` at 32 px and below.
- **Smallest horizontal lockup:** 120 px wide. Below that, use the icon alone.
- **Smallest stacked lockup:** 64 px wide.

## Don't

- Recolour or reorder the bottles, or give them different colours.
- Add outlines, shadows or glows to the bottles.
- Stretch the tile or change its corner radius.
- Use the full-size drawing below 32 px.
- Set the name in another typeface, or retype it instead of using the outlined files.
