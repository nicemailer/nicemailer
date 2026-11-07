# App icon: first proposals

Ten proposals for the icon of NiceMailer.app (November 2026), on the background rgb(138 165 129) (`#8AA581`), with `#F7F3E8` and `#2E4630`. They play on "Nm": the short name of Nice Mailer, and the symbol of the nanometre, a very short distance, with the sending of mail.

Each SVG is 1024 x 1024 with the macOS template: the shape is 824 x 824 at (100, 100), corner radius 185, the rest transparent.

| File | Idea |
| --- | --- |
| `1-envol.svg` | A paper plane leaves the m, with a dotted trail |
| `2-pied-a-coulisse.svg` | A caliper measures an envelope |
| `3-cote.svg` | A dimension line under Nm whose measure is an envelope |
| `4-timbre.svg` | A postage stamp with a 1 nm scale bar |
| `5-orbite.svg` | An envelope orbits Nm like an electron |
| `6-trait-continu.svg` | Nm written in one stroke, ending as a paper plane |
| `1a-envol-atomes.svg` | 1, flying over a lattice of atoms (a microscope image) |
| `1b-envol-trajectoire-graduee.svg` | 1, its trail graduated like a ruler |
| `1c-envol-un-nanometre.svg` | 1, a dimension line measures its flight: 1 nm |
| `1d-envol-barre-echelle.svg` | 1, with the scale bar of a micrograph in the corner |

The letters are text in Space Grotesk (DM Serif Display for 3): they render as designed only where the font is installed.

## The icon chosen: 1D

`1d-envol-barre-echelle.svg` is the icon of the app. `app/` holds it with its letters outlined, so that it depends on no font, and a simpler drawing for each small size, as Apple's icons do: at 128 px the "nm" of the scale bar goes, at 64 px the trail too, at 32 and 16 px only Nm and the bar stay.

| File | Used for |
| --- | --- |
| `app/AppIcon.svg` | 256 to 1024 px |
| `app/AppIcon-128.svg` | 128 px |
| `app/AppIcon-64.svg` | 64 px |
| `app/AppIcon-32.svg` | 16 and 32 px |
| `app/AppIcon.icns` | the icon of NiceMailer.app (`make bundle` copies it) |

To change it: edit the SVG with text, outline it with `scripts/outline-icon.py <SpaceGrotesk.ttf> <in.svg> <out.svg>` (fontTools, and the variable font of Google Fonts, OFL), then `scripts/make-icon.sh` (rsvg-convert, iconutil) makes the `.icns`.

The canvas where they were drawn, with their renderings at 128, 64 and 32 px: https://claude.ai/artifact/HztkR3vg6Tw7g4FR6irvRA (private).
